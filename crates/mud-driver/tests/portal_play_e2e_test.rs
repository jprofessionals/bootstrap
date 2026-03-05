//! Full-stack portal Play E2E test: testcontainers PostgreSQL + Rust driver
//! Server + real Ruby adapter process + Rust HTTP proxy + HTTP client.
//!
//! Tests the PlayApp web play flow: register, login, start play session,
//! execute game commands (look, movement, help, who), and access gating.
//! HTTP requests hit the Rust HTTP server, which proxies portal routes to the
//! Ruby adapter via a Unix domain socket.
//!
//! Run with: `cargo test -p mud-driver --test portal_play_e2e_test -- --ignored --nocapture`
//!
//! Prerequisites:
//! - Docker (for testcontainers PostgreSQL)
//! - Ruby installed and on PATH
//! - `bundle install` completed in `adapters/ruby/`

use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::Arc;
use std::time::Duration;

use mud_core::types::AreaId;
use mud_driver::config::{Config, DatabaseConfig, HttpConfig, WorldConfig};
use mud_driver::server::Server;
use mud_driver::web::server::{AppState, WebServer, init_templates};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::time::timeout;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Skip-condition helpers (same as portal_e2e_test.rs)
// ---------------------------------------------------------------------------

fn ruby_available() -> bool {
    StdCommand::new("ruby")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn adapter_ruby_dir() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest_dir)
        .join("../../adapters/ruby")
        .canonicalize()
        .expect("adapters/ruby directory must exist")
}

fn find_bundle_binary() -> PathBuf {
    if let Ok(output) = StdCommand::new("which").arg("bundle").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return PathBuf::from(path);
            }
        }
    }

    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        format!("{home}/.local/share/gem/ruby/3.4.0/bin/bundle"),
        format!("{home}/.local/share/gem/ruby/3.3.0/bin/bundle"),
        format!("{home}/.local/share/gem/ruby/3.2.0/bin/bundle"),
        format!("{home}/.gem/ruby/3.4.0/bin/bundle"),
        format!("{home}/.gem/ruby/3.3.0/bin/bundle"),
        "/usr/local/bin/bundle".into(),
        "/usr/bin/bundle".into(),
    ];

    for candidate in &candidates {
        let path = Path::new(candidate);
        if path.exists() {
            return path.to_path_buf();
        }
    }

    PathBuf::from("bundle")
}

fn bundle_available() -> bool {
    let ruby_dir = adapter_ruby_dir();
    let bundle_bin = find_bundle_binary();

    let result = StdCommand::new(&bundle_bin)
        .args(["exec", "ruby", "-e", "require 'msgpack'; puts 'ok'"])
        .current_dir(&ruby_dir)
        .output();

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout.trim() == "ok"
        }
        Err(_) => false,
    }
}

fn prerequisites_met() -> bool {
    if !ruby_available() {
        eprintln!("SKIPPED: Ruby not available on PATH");
        return false;
    }
    if !bundle_available() {
        eprintln!(
            "SKIPPED: Ruby gems not installed. Run `bundle install` in adapters/ruby/"
        );
        return false;
    }
    true
}

// ---------------------------------------------------------------------------
// Ruby adapter process wrapper
// ---------------------------------------------------------------------------

struct RubyAdapterProcess {
    _child: tokio::process::Child,
}

impl RubyAdapterProcess {
    fn spawn(socket_path: &Path, web_socket_path: &Path, world_path: &Path) -> Self {
        let ruby_dir = adapter_ruby_dir();
        let adapter_bin = ruby_dir.join("bin/mud-adapter");
        let bundle_bin = find_bundle_binary();

        let child = tokio::process::Command::new(&bundle_bin)
            .arg("exec")
            .arg("ruby")
            .arg(&adapter_bin)
            .arg("--socket")
            .arg(socket_path)
            .arg("--web-socket")
            .arg(web_socket_path)
            .current_dir(&ruby_dir)
            .env("BUNDLE_GEMFILE", ruby_dir.join("Gemfile"))
            .env("BUNDLE_PATH", ruby_dir.join("vendor/bundle"))
            .env("MUD_WORLD_PATH", world_path)
            .kill_on_drop(true)
            .stderr(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("spawn Ruby adapter process");

        Self { _child: child }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find a free TCP port by binding to port 0.
fn find_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Poll the web server until the portal is ready (proxied through Rust HTTP).
/// Waits until a portal route returns a non-502 response.
async fn wait_for_portal(port: u16, timeout_duration: Duration) {
    let url = format!("http://127.0.0.1:{}/account/login", port);
    let deadline = tokio::time::Instant::now() + timeout_duration;

    loop {
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "Portal not ready on port {} after {:?}",
                port, timeout_duration
            );
        }
        match reqwest::get(&url).await {
            Ok(resp) if resp.status() != 502 => return,
            _ => tokio::time::sleep(Duration::from_millis(200)).await,
        }
    }
}

/// Copy the demo area from the project's world/ directory into a temp world dir.
fn setup_demo_world(world_path: &Path) {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let source = Path::new(manifest_dir)
        .join("../../world/test/demo")
        .canonicalize()
        .expect("world/test/demo must exist");

    let dest = world_path.join("test").join("demo");
    std::fs::create_dir_all(dest.join("rooms")).expect("create demo rooms dir");

    // Copy .meta.yml
    std::fs::copy(source.join(".meta.yml"), dest.join(".meta.yml"))
        .expect("copy .meta.yml");

    // Copy mud_aliases.rb if it exists
    if source.join("mud_aliases.rb").exists() {
        std::fs::copy(source.join("mud_aliases.rb"), dest.join("mud_aliases.rb"))
            .expect("copy mud_aliases.rb");
    }

    // Copy room files
    for entry in std::fs::read_dir(source.join("rooms")).expect("read rooms dir") {
        let entry = entry.expect("read room entry");
        let dest_file = dest.join("rooms").join(entry.file_name());
        std::fs::copy(entry.path(), &dest_file).expect("copy room file");
    }
}

// ===========================================================================
// Tests
// ===========================================================================

/// Full portal Play E2E test.
///
/// Phase 1: Register + Login
/// Phase 2: Start play session
/// Phase 3: Game commands (look, north, south, help, who)
/// Phase 4: Access gating
#[tokio::test]
#[ignore] // Requires Docker + Ruby; run with: cargo test -- --ignored
async fn portal_play_e2e() {
    if !prerequisites_met() {
        return;
    }

    eprintln!("[portal-play-e2e] Starting play E2E test");

    // ===================================================================
    // Phase 0: Setup
    // ===================================================================

    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let socket_path = temp_dir.path().join("portal-play-e2e.sock");
    let http_port = find_free_port();
    let web_socket_path = temp_dir.path().join("portal-web.sock");

    // --- PostgreSQL via testcontainers ---
    eprintln!("[portal-play-e2e] Starting PostgreSQL container...");
    let container = Postgres::default()
        .start()
        .await
        .expect("start PostgreSQL container");

    let host_port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("get PostgreSQL port");

    eprintln!("[portal-play-e2e] PostgreSQL ready on port {}", host_port);

    // --- Create temp directories for world and repos ---
    let repos_path = temp_dir.path().join("repos");
    std::fs::create_dir_all(&repos_path).expect("create repos dir");
    let world_path = temp_dir.path().join("world");
    std::fs::create_dir_all(&world_path).expect("create world dir");

    // --- Copy demo area into temp world ---
    setup_demo_world(&world_path);
    eprintln!("[portal-play-e2e] Demo area copied to temp world");

    // --- Rust Server with Config ---
    let mut config = Config::default();
    config.database = DatabaseConfig {
        host: "127.0.0.1".into(),
        port: host_port,
        admin_user: "postgres".into(),
        admin_password: Some("postgres".into()),
        ..DatabaseConfig::default()
    };
    config.world = WorldConfig {
        data_path: temp_dir.path().to_str().unwrap().into(),
        path: world_path.to_str().unwrap().into(),
        git_path: repos_path.to_str().unwrap().into(),
    };
    let mut server = Server::new_with_socket_path(config, socket_path.clone());

    let listener = server
        .start_adapter_manager()
        .await
        .expect("start adapter manager");
    eprintln!("[portal-play-e2e] Adapter manager started");

    // --- Spawn Ruby adapter ---
    let _ruby_process = RubyAdapterProcess::spawn(&socket_path, &web_socket_path, &world_path);
    eprintln!(
        "[portal-play-e2e] Ruby adapter spawned (web socket {:?})",
        web_socket_path
    );

    // --- Accept adapter connection (handshake) ---
    let language = timeout(STARTUP_TIMEOUT, server.accept_adapter(&listener))
        .await
        .expect("accept adapter timed out")
        .expect("accept adapter failed");
    assert_eq!(language, "ruby");
    eprintln!("[portal-play-e2e] Ruby adapter connected");

    // --- Database setup + send configure to Ruby ---
    server
        .setup_database()
        .await
        .expect("setup database");
    eprintln!("[portal-play-e2e] Database initialized");

    // --- Start Rust HTTP server (proxies portal routes to Ruby via Unix socket) ---
    let player_store = server.player_store().expect("player_store after setup").clone();
    let repo_manager = server.repo_manager().expect("repo_manager after setup").clone();
    let workspace = server.workspace().expect("workspace after setup").clone();
    let templates = Arc::new(init_templates().expect("init templates"));
    let state = AppState {
        player_store,
        repo_manager,
        workspace,
        templates,
        ai_key_store: None,
        skills_service: None,
        http_client: reqwest::Client::new(),
        portal_socket: web_socket_path.to_str().unwrap().to_string(),
        anthropic_base_url: None,
    };
    let http_config = HttpConfig {
        host: "127.0.0.1".into(),
        port: http_port,
        enabled: true,
        ..Default::default()
    };
    let web_server = WebServer::new(http_config, state);
    let _http_handle = tokio::spawn(async move {
        if let Err(e) = web_server.start().await {
            eprintln!("[portal-play-e2e] HTTP server error: {}", e);
        }
    });

    // --- Load the demo area ---
    let demo_area_path = world_path
        .join("test")
        .join("demo")
        .to_str()
        .unwrap()
        .to_string();
    server
        .send_load_area(AreaId::new("test", "demo"), demo_area_path)
        .await
        .expect("send load area");
    eprintln!("[portal-play-e2e] Demo area loaded");

    // Give the adapter a moment to process the load_area message
    tokio::time::sleep(Duration::from_millis(500)).await;

    // --- Run server MOP message loop in background ---
    let bg_handle = tokio::spawn(async move {
        loop {
            match server.recv_adapter_message().await {
                Some(msg) => server.handle_adapter_message(msg).await,
                None => break,
            }
        }
    });

    // --- Wait for portal to be ready (Ruby via Rust proxy) ---
    eprintln!(
        "[portal-play-e2e] Waiting for portal on Rust HTTP port {}...",
        http_port
    );
    wait_for_portal(http_port, STARTUP_TIMEOUT).await;
    eprintln!("[portal-play-e2e] Portal ready");

    let base_url = format!("http://127.0.0.1:{}", http_port);

    // HTTP client with cookie jar (no auto-redirect so we can verify Location headers)
    let client = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    // ===================================================================
    // Phase 1: Register + Login
    // ===================================================================
    eprintln!("[portal-play-e2e] Phase 1: Register + Login");

    let resp = client
        .post(format!("{}/account/register", base_url))
        .form(&[
            ("username", "alice"),
            ("password", "secret123"),
            ("character", "Warrior"),
        ])
        .send()
        .await
        .expect("POST register alice");
    let status = resp.status();
    let location = resp
        .headers()
        .get("location")
        .map(|v| v.to_str().unwrap_or("").to_string());
    assert_eq!(
        status, 302,
        "register should redirect. Location: {:?}",
        location
    );
    eprintln!("[portal-play-e2e]   alice registered (302 redirect)");

    // Follow redirect to establish session
    if let Some(loc) = &location {
        let redirect_url = if loc.starts_with('/') {
            format!("{}{}", base_url, loc)
        } else {
            loc.clone()
        };
        let resp = client
            .get(&redirect_url)
            .send()
            .await
            .expect("GET redirect after register");
        assert_eq!(resp.status(), 200, "should land on page after register");
    }

    // ===================================================================
    // Phase 2: Start play session
    // ===================================================================
    eprintln!("[portal-play-e2e] Phase 2: Start play session");

    let resp = client
        .post(format!("{}/play/start", base_url))
        .send()
        .await
        .expect("POST /play/start");
    assert_eq!(resp.status(), 200, "POST /play/start should return 200");
    let body = resp.text().await.unwrap();
    eprintln!("[portal-play-e2e]   POST /play/start response: {}", &body[..body.len().min(300)]);

    let json: serde_json::Value = serde_json::from_str(&body).expect("parse JSON");
    let output = json["output"].as_str().expect("output field");
    assert!(
        output.contains("Welcome"),
        "start output should contain 'Welcome'. Got: {}",
        &output[..output.len().min(200)]
    );
    assert!(
        output.contains("The Entrance"),
        "start output should contain 'The Entrance'. Got: {}",
        &output[..output.len().min(200)]
    );
    assert!(
        output.contains("north"),
        "start output should mention 'north' exit. Got: {}",
        &output[..output.len().min(200)]
    );
    eprintln!("[portal-play-e2e]   Play session started OK (Welcome + room description)");

    // ===================================================================
    // Phase 3: Game commands
    // ===================================================================
    eprintln!("[portal-play-e2e] Phase 3: Game commands");

    // --- look ---
    let resp = client
        .post(format!("{}/play/command", base_url))
        .header("Content-Type", "application/json")
        .body(r#"{"input": "look"}"#)
        .send()
        .await
        .expect("POST /play/command look");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let output = json["output"].as_str().unwrap();
    assert!(
        output.contains("The Entrance"),
        "look should show 'The Entrance'. Got: {}",
        output
    );
    eprintln!("[portal-play-e2e]   look: OK (The Entrance)");

    // --- north ---
    let resp = client
        .post(format!("{}/play/command", base_url))
        .header("Content-Type", "application/json")
        .body(r#"{"input": "north"}"#)
        .send()
        .await
        .expect("POST /play/command north");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let output = json["output"].as_str().unwrap();
    assert!(
        output.contains("The Garden"),
        "north should show 'The Garden'. Got: {}",
        output
    );
    eprintln!("[portal-play-e2e]   north: OK (The Garden)");

    // --- look (should now be in garden) ---
    let resp = client
        .post(format!("{}/play/command", base_url))
        .header("Content-Type", "application/json")
        .body(r#"{"input": "look"}"#)
        .send()
        .await
        .expect("POST /play/command look in garden");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let output = json["output"].as_str().unwrap();
    assert!(
        output.contains("The Garden"),
        "look in garden should show 'The Garden'. Got: {}",
        output
    );
    eprintln!("[portal-play-e2e]   look (garden): OK (The Garden)");

    // --- south (back to entrance) ---
    let resp = client
        .post(format!("{}/play/command", base_url))
        .header("Content-Type", "application/json")
        .body(r#"{"input": "south"}"#)
        .send()
        .await
        .expect("POST /play/command south");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let output = json["output"].as_str().unwrap();
    assert!(
        output.contains("The Entrance"),
        "south should show 'The Entrance'. Got: {}",
        output
    );
    eprintln!("[portal-play-e2e]   south: OK (The Entrance)");

    // --- help ---
    let resp = client
        .post(format!("{}/play/command", base_url))
        .header("Content-Type", "application/json")
        .body(r#"{"input": "help"}"#)
        .send()
        .await
        .expect("POST /play/command help");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let output = json["output"].as_str().unwrap();
    assert!(
        output.contains("Available commands"),
        "help should show 'Available commands'. Got: {}",
        output
    );
    eprintln!("[portal-play-e2e]   help: OK (Available commands)");

    // --- who ---
    let resp = client
        .post(format!("{}/play/command", base_url))
        .header("Content-Type", "application/json")
        .body(r#"{"input": "who"}"#)
        .send()
        .await
        .expect("POST /play/command who");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let output = json["output"].as_str().unwrap();
    assert!(
        output.contains("Players online") || output.contains("Warrior"),
        "who should show players. Got: {}",
        output
    );
    eprintln!("[portal-play-e2e]   who: OK (Players online)");

    // ===================================================================
    // Phase 4: Access gating
    // ===================================================================
    eprintln!("[portal-play-e2e] Phase 4: Access gating");

    // Unauthenticated GET /play/ should redirect to /account/login
    let anon_client = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    let resp = anon_client
        .get(format!("{}/play/", base_url))
        .send()
        .await
        .expect("GET /play/ unauthenticated");
    assert_eq!(
        resp.status(),
        302,
        "GET /play/ unauthenticated should redirect (302)"
    );
    let location = resp
        .headers()
        .get("location")
        .expect("/play/ redirect should have Location header")
        .to_str()
        .unwrap();
    assert!(
        location.ends_with("/account/login"),
        "GET /play/ unauthenticated should redirect to /account/login, got: {}",
        location
    );
    eprintln!("[portal-play-e2e]   GET /play/ unauthenticated: OK (302 -> /account/login)");

    // ===================================================================
    // Cleanup
    // ===================================================================
    eprintln!("[portal-play-e2e] Cleaning up...");
    bg_handle.abort();
    let _ = bg_handle.await;
    // _ruby_process, _container, temp_dir cleaned up on drop
    eprintln!("[portal-play-e2e] Portal play E2E test passed!");
}
