//! E2E test: registering a new account auto-creates a default git area
//! and the seeded files are visible in the editor.
//!
//! HTTP requests hit the Rust HTTP server, which proxies portal routes to the
//! Ruby adapter via a Unix domain socket. The Ruby adapter communicates with
//! the Rust driver via MOP for account operations backed by PostgreSQL.
//!
//! Run with: `cargo test -p mud-driver --test portal_default_area_e2e_test -- --ignored --nocapture`
//!
//! Prerequisites:
//! - Docker (for testcontainers PostgreSQL)
//! - Ruby installed and on PATH
//! - `bundle install` completed in `adapters/ruby/`

use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::Arc;
use std::time::Duration;

use mud_driver::config::{Config, DatabaseConfig, HttpConfig, WorldConfig};
use mud_driver::server::Server;
use mud_driver::web::server::{AppState, WebServer, init_templates};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::time::timeout;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Skip-condition helpers
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

// ===========================================================================
// Test
// ===========================================================================

/// Registering a new account automatically creates a default git area and
/// the seeded template files are visible through the editor API.
///
/// Phase 0: Setup (testcontainers PG + Server + Ruby adapter)
/// Phase 1: Register alice (builder by default, auto-creates area)
/// Phase 2: Verify repo appears in git API
/// Phase 3: Verify editor can list and read the seeded files
#[tokio::test]
#[ignore] // Requires Docker + Ruby; run with: cargo test -- --ignored
async fn register_creates_default_area_visible_in_editor() {
    if !prerequisites_met() {
        return;
    }

    eprintln!("[default-area-e2e] Starting default area E2E test");

    // ===================================================================
    // Phase 0: Setup
    // ===================================================================

    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let socket_path = temp_dir.path().join("default-area-e2e.sock");
    let web_socket_path = temp_dir.path().join("portal-web.sock");
    let http_port = find_free_port();

    // --- PostgreSQL via testcontainers ---
    eprintln!("[default-area-e2e] Starting PostgreSQL container...");
    let container = Postgres::default()
        .start()
        .await
        .expect("start PostgreSQL container");

    let host_port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("get PostgreSQL port");

    eprintln!("[default-area-e2e] PostgreSQL ready on port {}", host_port);

    // --- Create temp directories for world and repos ---
    let repos_path = temp_dir.path().join("repos");
    std::fs::create_dir_all(&repos_path).expect("create repos dir");
    let world_path = temp_dir.path().join("world");
    std::fs::create_dir_all(&world_path).expect("create world dir");

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
    eprintln!("[default-area-e2e] Adapter manager started");

    // --- Spawn Ruby adapter ---
    let _ruby_process =
        RubyAdapterProcess::spawn(&socket_path, &web_socket_path, &temp_dir.path().join("world"));
    eprintln!(
        "[default-area-e2e] Ruby adapter spawned (web socket {:?})",
        web_socket_path
    );

    // --- Accept adapter connection (handshake) ---
    let language = timeout(STARTUP_TIMEOUT, server.accept_adapter(&listener))
        .await
        .expect("accept adapter timed out")
        .expect("accept adapter failed");
    assert_eq!(language, "ruby");
    eprintln!("[default-area-e2e] Ruby adapter connected");

    // --- Database setup + send configure to Ruby ---
    server
        .setup_database()
        .await
        .expect("setup database");
    eprintln!("[default-area-e2e] Database initialized");

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
            eprintln!("[default-area-e2e] HTTP server error: {}", e);
        }
    });

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
        "[default-area-e2e] Waiting for portal on Rust HTTP port {}...",
        http_port
    );
    wait_for_portal(http_port, STARTUP_TIMEOUT).await;
    eprintln!("[default-area-e2e] Portal ready");

    let base_url = format!("http://127.0.0.1:{}", http_port);

    let client = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    // ===================================================================
    // Phase 1: Register alice — builder by default, area auto-created
    // ===================================================================
    eprintln!("[default-area-e2e] Phase 1: Register alice (auto-creates area)");

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
    assert_eq!(resp.status(), 302, "register should redirect");

    // Follow redirect to establish session
    let location = resp
        .headers()
        .get("location")
        .map(|v| v.to_str().unwrap_or("").to_string());
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
    eprintln!("[default-area-e2e]   alice registered (builder by default)");

    // ===================================================================
    // Phase 2: Verify repo appears in git API (no manual create_repo!)
    // ===================================================================
    eprintln!("[default-area-e2e] Phase 2: Verify auto-created repo in git API");

    // GET /git/api/repos — should list "alice" repo
    let resp = client
        .get(format!("{}/git/api/repos", base_url))
        .send()
        .await
        .expect("GET /git/api/repos");
    assert_eq!(resp.status(), 200, "GET /git/api/repos should return 200");
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("alice"),
        "GET /git/api/repos should contain 'alice' (auto-created repo). Got:\n{}",
        &body[..body.len().min(500)]
    );
    eprintln!("[default-area-e2e]   GET /git/api/repos: OK (contains 'alice')");

    // ===================================================================
    // Phase 3: Verify editor can list and read seeded files
    // ===================================================================
    eprintln!("[default-area-e2e] Phase 3: Editor file listing");

    // GET /editor/ — dashboard should load
    let resp = client
        .get(format!("{}/editor/", base_url))
        .send()
        .await
        .expect("GET /editor/");
    assert_eq!(resp.status(), 200, "GET /editor/ should return 200");
    eprintln!("[default-area-e2e]   GET /editor/: OK (200)");

    // GET /editor/api/files?repo=alice/alice — list files in the auto-created area
    let resp = client
        .get(format!(
            "{}/editor/api/files?repo=alice/alice",
            base_url
        ))
        .send()
        .await
        .expect("GET /editor/api/files");
    assert_eq!(
        resp.status(),
        200,
        "GET /editor/api/files should return 200"
    );
    let body = resp.text().await.unwrap();
    eprintln!("[default-area-e2e]   File listing response: {}", &body[..body.len().min(500)]);

    // Verify seeded template files are present
    // (.meta.yml is a dotfile, hidden by Ruby's Dir.glob by default)
    assert!(
        body.contains("mud_aliases.rb"),
        "File listing should contain 'mud_aliases.rb'. Got:\n{}",
        &body[..body.len().min(500)]
    );
    assert!(
        body.contains("entrance.rb"),
        "File listing should contain 'entrance.rb'. Got:\n{}",
        &body[..body.len().min(500)]
    );
    eprintln!("[default-area-e2e]   GET /editor/api/files: OK (contains seeded files)");

    // GET /editor/api/files/rooms/entrance.rb?repo=alice/alice — read a seeded file
    let resp = client
        .get(format!(
            "{}/editor/api/files/rooms/entrance.rb?repo=alice/alice",
            base_url
        ))
        .send()
        .await
        .expect("GET entrance.rb");
    assert_eq!(resp.status(), 200, "GET entrance.rb should return 200");
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("class Entrance"),
        "entrance.rb should contain 'class Entrance'. Got:\n{}",
        &body[..body.len().min(500)]
    );
    eprintln!("[default-area-e2e]   GET entrance.rb: OK (contains 'class Entrance')");

    // GET /editor/api/files/mud_aliases.rb?repo=alice/alice — read seeded aliases file
    let resp = client
        .get(format!(
            "{}/editor/api/files/mud_aliases.rb?repo=alice/alice",
            base_url
        ))
        .send()
        .await
        .expect("GET mud_aliases.rb");
    assert_eq!(resp.status(), 200, "GET mud_aliases.rb should return 200");
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("Room"),
        "mud_aliases.rb should contain 'Room' alias. Got:\n{}",
        &body[..body.len().min(500)]
    );
    eprintln!("[default-area-e2e]   GET mud_aliases.rb: OK (contains Room alias)");

    // ===================================================================
    // Cleanup
    // ===================================================================
    eprintln!("[default-area-e2e] Cleaning up...");
    bg_handle.abort();
    let _ = bg_handle.await;
    eprintln!("[default-area-e2e] Default area E2E test passed!");
}
