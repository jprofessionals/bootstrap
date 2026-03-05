//! E2E test for BuilderApp — serving area web content.
//!
//! Tests ERB rendering, static file serving, branch access control,
//! and hot-reload after git commits.
//!
//! HTTP requests hit the Rust HTTP server, which proxies portal routes to the
//! Ruby adapter via a Unix domain socket.
//!
//! Run with: `cargo test -p mud-driver --test portal_builder_e2e_test -- --ignored --nocapture`
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

/// Poll the Rust HTTP server until the portal is ready (proxied through to Ruby).
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

/// Wait for builder page to return 200 with actual area content.
/// Polls until the response body contains the expected area name,
/// indicating the area has been loaded by the adapter.
async fn wait_for_builder_content(
    client: &reqwest::Client,
    url: &str,
    expected_content: &str,
    timeout_duration: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout_duration;

    loop {
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "Builder page at {} never contained '{}' after {:?}",
                url, expected_content, timeout_duration
            );
        }
        match client.get(url).send().await {
            Ok(resp) if resp.status() == 200 => {
                let body = resp.text().await.unwrap_or_default();
                if body.contains(expected_content) {
                    return;
                }
            }
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

// ===========================================================================
// Test
// ===========================================================================

/// Full BuilderApp E2E test:
///
/// Phase 0: Setup (testcontainers PG + Server + Ruby adapter)
/// Phase 1: Register alice (builder, auto-creates area alice/alice)
/// Phase 2: Commit to trigger area loading
/// Phase 3: GET /builder/alice/alice/ — verify ERB-rendered content
/// Phase 4: GET /builder/alice/alice/web/style.css — verify static file serving
/// Phase 5: GET /builder/alice/alice@dev/ unauthenticated — verify 302 redirect
/// Phase 6: GET /builder/alice/alice@dev/ authenticated — verify 200
/// Phase 7: Edit web/index.erb + commit — verify hot-reload updates content
/// Phase 8: GET /builder/ — verify area index lists alice/alice
#[tokio::test]
#[ignore] // Requires Docker + Ruby; run with: cargo test -- --ignored
async fn builder_app_e2e() {
    if !prerequisites_met() {
        return;
    }

    eprintln!("[builder-e2e] Starting BuilderApp E2E test");

    // ===================================================================
    // Phase 0: Setup
    // ===================================================================

    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let socket_path = temp_dir.path().join("builder-e2e.sock");
    let http_port = find_free_port();
    let web_socket_path = temp_dir.path().join("portal-web.sock");

    eprintln!("[builder-e2e] Starting PostgreSQL container...");
    let container = Postgres::default()
        .start()
        .await
        .expect("start PostgreSQL container");

    let host_port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("get PostgreSQL port");

    eprintln!("[builder-e2e] PostgreSQL ready on port {}", host_port);

    let repos_path = temp_dir.path().join("repos");
    std::fs::create_dir_all(&repos_path).expect("create repos dir");
    let world_path = temp_dir.path().join("world");
    std::fs::create_dir_all(&world_path).expect("create world dir");

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

    let _ruby_process =
        RubyAdapterProcess::spawn(&socket_path, &web_socket_path, &world_path);

    let language = timeout(STARTUP_TIMEOUT, server.accept_adapter(&listener))
        .await
        .expect("accept adapter timed out")
        .expect("accept adapter failed");
    assert_eq!(language, "ruby");

    server
        .setup_database()
        .await
        .expect("setup database");
    eprintln!("[builder-e2e] Database initialized");

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
            eprintln!("[builder-e2e] HTTP server error: {}", e);
        }
    });

    let bg_handle = tokio::spawn(async move {
        loop {
            match server.recv_adapter_message().await {
                Some(msg) => server.handle_adapter_message(msg).await,
                None => break,
            }
        }
    });

    wait_for_portal(http_port, STARTUP_TIMEOUT).await;
    eprintln!("[builder-e2e] Portal ready (Rust HTTP proxy + Ruby backend)");

    let base_url = format!("http://127.0.0.1:{}", http_port);

    let client = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    // ===================================================================
    // Phase 1: Register alice (builder, auto-creates area alice/alice)
    // ===================================================================
    eprintln!("[builder-e2e] Phase 1: Register alice");

    let resp = client
        .post(format!("{}/account/register", base_url))
        .form(&[
            ("username", "alice"),
            ("password", "secret123"),
            ("character", "Warrior"),
        ])
        .send()
        .await
        .expect("POST register");
    assert_eq!(resp.status(), 302, "register should redirect");

    // Follow redirect to establish session
    if let Some(loc) = resp.headers().get("location") {
        let loc = loc.to_str().unwrap();
        let url = if loc.starts_with('/') {
            format!("{}{}", base_url, loc)
        } else {
            loc.to_string()
        };
        client.get(&url).send().await.expect("follow redirect");
    }
    eprintln!("[builder-e2e]   alice registered");

    // ===================================================================
    // Phase 2: Trigger area load via commit
    // ===================================================================
    // In production, server.run() calls load_areas() at startup.
    // In the test, areas aren't loaded until a commit triggers area_reload.
    // Edit a file and commit to trigger the area loading.
    eprintln!("[builder-e2e] Phase 2: Trigger area load via commit");

    let resp = client
        .put(format!(
            "{}/editor/api/files/rooms/entrance.rb?repo=alice/alice",
            base_url
        ))
        .header("Content-Type", "application/json")
        .body(r#"{"content": "class Entrance < Room\n  title \"The Entrance\"\n  description \"Welcome to alice.\"\nend\n"}"#)
        .send()
        .await
        .expect("PUT entrance.rb");
    assert_eq!(resp.status(), 200, "PUT entrance.rb should succeed");

    let resp = client
        .post(format!("{}/git/api/repos/alice/alice/commit", base_url))
        .header("Content-Type", "application/json")
        .body(r#"{"message": "Initial commit to trigger area load"}"#)
        .send()
        .await
        .expect("POST commit");
    assert_eq!(resp.status(), 200, "commit should succeed");
    eprintln!("[builder-e2e]   commit triggered area reload");

    // Small delay for reload to propagate
    tokio::time::sleep(Duration::from_millis(500)).await;

    // ===================================================================
    // Phase 3: GET /builder/alice/alice/ — ERB-rendered content (public)
    // ===================================================================
    eprintln!("[builder-e2e] Phase 3: Builder ERB rendering (main branch, public)");

    // Wait for builder page to show area data (proves area is loaded)
    let builder_url = format!("{}/builder/alice/alice/", base_url);
    wait_for_builder_content(&client, &builder_url, "Rooms: 1", Duration::from_secs(15)).await;

    let resp = client
        .get(&builder_url)
        .send()
        .await
        .expect("GET /builder/alice/alice/");
    assert_eq!(resp.status(), 200, "builder main branch should return 200");
    let body = resp.text().await.unwrap();

    // The template web/index.erb contains: area_name, room_count, item_count, npc_count
    assert!(
        body.contains("alice"),
        "builder page should contain area name 'alice'. Got:\n{}",
        &body[..body.len().min(500)]
    );
    assert!(
        body.contains("Rooms:"),
        "builder page should contain 'Rooms:' from template. Got:\n{}",
        &body[..body.len().min(500)]
    );
    eprintln!("[builder-e2e]   ERB rendering OK: contains area data");

    // ===================================================================
    // Phase 4: GET /builder/alice/alice/web/style.css — static file
    // ===================================================================
    eprintln!("[builder-e2e] Phase 4: Static file serving");

    let resp = client
        .get(format!("{}/builder/alice/alice/web/style.css", base_url))
        .send()
        .await
        .expect("GET style.css");
    assert_eq!(resp.status(), 200, "style.css should return 200");
    let content_type = resp
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap_or(""))
        .unwrap_or("");
    assert!(
        content_type.contains("text/css"),
        "style.css should have text/css content type. Got: {}",
        content_type
    );
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("font-family"),
        "style.css should contain CSS content. Got:\n{}",
        &body[..body.len().min(500)]
    );
    eprintln!("[builder-e2e]   style.css served OK");

    // ===================================================================
    // Phase 5: GET /builder/alice/alice@dev/ unauthenticated — 302
    // ===================================================================
    eprintln!("[builder-e2e] Phase 5: Branch access control (unauthenticated)");

    // Use a separate client without cookies
    let anon_client = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    let resp = anon_client
        .get(format!("{}/builder/alice/alice@dev/", base_url))
        .send()
        .await
        .expect("GET /builder/alice/alice@dev/ (anon)");
    assert_eq!(
        resp.status(),
        302,
        "unauthenticated access to @dev should redirect. Got: {}",
        resp.status()
    );
    let location = resp
        .headers()
        .get("location")
        .map(|v| v.to_str().unwrap_or(""))
        .unwrap_or("");
    assert!(
        location.contains("/account/login"),
        "should redirect to login. Got location: {}",
        location
    );
    eprintln!("[builder-e2e]   unauthenticated @dev → 302 to login");

    // ===================================================================
    // Phase 6: GET /builder/alice/alice@dev/ authenticated — 200
    // ===================================================================
    eprintln!("[builder-e2e] Phase 6: Branch access (authenticated)");

    let resp = client
        .get(format!("{}/builder/alice/alice@dev/", base_url))
        .send()
        .await
        .expect("GET /builder/alice/alice@dev/ (authenticated)");
    assert_eq!(
        resp.status(),
        200,
        "authenticated access to @dev should return 200. Got: {}",
        resp.status()
    );
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("alice"),
        "authenticated @dev builder page should contain area name. Got:\n{}",
        &body[..body.len().min(500)]
    );
    eprintln!("[builder-e2e]   authenticated @dev → 200 OK");

    // ===================================================================
    // Phase 7: Edit web/index.erb + commit — verify hot-reload
    // ===================================================================
    eprintln!("[builder-e2e] Phase 7: Hot-reload after commit");

    // Update web/index.erb via the editor API
    let resp = client
        .put(format!(
            "{}/editor/api/files/web/index.erb?repo=alice/alice",
            base_url
        ))
        .header("Content-Type", "application/json")
        .body(r#"{"content": "<!DOCTYPE html>\n<html>\n<head><title>Updated <%= area_name %></title></head>\n<body>\n  <h1>UPDATED: <%= area_name %></h1>\n  <p>Rooms: <%= room_count %></p>\n</body>\n</html>\n"}"#)
        .send()
        .await
        .expect("PUT web/index.erb");
    assert_eq!(resp.status(), 200, "PUT web/index.erb should succeed");
    eprintln!("[builder-e2e]   web/index.erb updated via editor");

    // Commit (triggers area reload)
    let resp = client
        .post(format!(
            "{}/git/api/repos/alice/alice/commit",
            base_url
        ))
        .header("Content-Type", "application/json")
        .body(r#"{"message": "Update builder web template"}"#)
        .send()
        .await
        .expect("POST commit");
    assert_eq!(resp.status(), 200, "commit should succeed");
    eprintln!("[builder-e2e]   committed (triggers area reload)");

    // Small delay for reload to propagate
    tokio::time::sleep(Duration::from_millis(500)).await;

    // GET @dev builder page again — should reflect updated template
    let resp = client
        .get(format!("{}/builder/alice/alice@dev/", base_url))
        .send()
        .await
        .expect("GET /builder/alice/alice@dev/ (after reload)");
    assert_eq!(resp.status(), 200, "builder should return 200 after reload");
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("UPDATED:"),
        "builder page should contain 'UPDATED:' after hot-reload. Got:\n{}",
        &body[..body.len().min(500)]
    );
    eprintln!("[builder-e2e]   hot-reload OK: page shows updated content");

    // ===================================================================
    // Phase 8: GET /builder/ — area index
    // ===================================================================
    eprintln!("[builder-e2e] Phase 8: Builder area index");

    let resp = client
        .get(format!("{}/builder/", base_url))
        .send()
        .await
        .expect("GET /builder/");
    assert_eq!(resp.status(), 200, "builder index should return 200");
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("alice/alice"),
        "builder index should list alice/alice area. Got:\n{}",
        &body[..body.len().min(500)]
    );
    eprintln!("[builder-e2e]   builder index lists alice/alice");

    // ===================================================================
    // Cleanup
    // ===================================================================
    eprintln!("[builder-e2e] Cleaning up...");
    bg_handle.abort();
    let _ = bg_handle.await;
    eprintln!("[builder-e2e] BuilderApp E2E test passed!");
}
