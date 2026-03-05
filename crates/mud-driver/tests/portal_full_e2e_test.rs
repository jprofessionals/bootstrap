//! Full-stack portal E2E test: testcontainers PostgreSQL + Rust driver Server
//! + real Ruby adapter process + Rust HTTP proxy + HTTP client.
//!
//! Tests the GitApp, EditorApp, and access-gating flows through the actual
//! web interface.  HTTP requests hit the Ruby adapter's portal web server,
//! which communicates with the Rust driver via MOP for account operations
//! and git/editor API calls backed by PostgreSQL and the filesystem.
//!
//! Run with: `cargo test -p mud-driver --test portal_full_e2e_test -- --ignored --nocapture`
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
use mud_driver::web::server::{AppState, WebServer, init_templates};
use mud_driver::server::Server;
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

/// Poll the portal until it responds (ignoring 502 Bad Gateway while Ruby starts).
async fn wait_for_portal(port: u16, timeout_duration: Duration) {
    let url = format!("http://127.0.0.1:{}/account/login", port);
    let deadline = tokio::time::Instant::now() + timeout_duration;
    loop {
        if tokio::time::Instant::now() >= deadline {
            panic!("Portal not ready on port {} after {:?}", port, timeout_duration);
        }
        match reqwest::get(&url).await {
            Ok(resp) if resp.status() != 502 => return,
            _ => tokio::time::sleep(Duration::from_millis(200)).await,
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

/// Full portal E2E test covering GitApp, EditorApp, and access-gating.
///
/// Phase 0: Setup (testcontainers PG + Server + Ruby adapter)
/// Phase 1: Register alice + promote to builder
/// Phase 2: Create repo via direct API
/// Phase 3: GitApp API tests
/// Phase 4: EditorApp tests
/// Phase 5: Access gating tests (unauthenticated + non-builder)
#[tokio::test]
#[ignore] // Requires Docker + Ruby; run with: cargo test -- --ignored
async fn portal_gitapp_editor_and_access_e2e() {
    if !prerequisites_met() {
        return;
    }

    eprintln!("[portal-full-e2e] Starting full-stack portal E2E test");

    // ===================================================================
    // Phase 0: Setup
    // ===================================================================

    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let socket_path = temp_dir.path().join("portal-full-e2e.sock");
    let http_port = find_free_port();
    let web_socket_path = temp_dir.path().join("portal-web.sock");

    // --- PostgreSQL via testcontainers ---
    eprintln!("[portal-full-e2e] Starting PostgreSQL container...");
    let container = Postgres::default()
        .start()
        .await
        .expect("start PostgreSQL container");

    let host_port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("get PostgreSQL port");

    eprintln!("[portal-full-e2e] PostgreSQL ready on port {}", host_port);

    // --- Create temp directories for world and repos ---
    let repos_path = temp_dir.path().join("repos");
    std::fs::create_dir_all(&repos_path).expect("create repos dir");
    let world_path = temp_dir.path().join("world");
    std::fs::create_dir_all(&world_path).expect("create world dir");

    // --- Rust Server with Config (same initialization as `just up`) ---
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
    eprintln!("[portal-full-e2e] Adapter manager started");

    // --- Spawn Ruby adapter ---
    let _ruby_process = RubyAdapterProcess::spawn(&socket_path, &web_socket_path, &temp_dir.path().join("world"));
    eprintln!(
        "[portal-full-e2e] Ruby adapter spawned (web socket {:?})",
        web_socket_path
    );

    // --- Accept adapter connection (handshake) ---
    let language = timeout(STARTUP_TIMEOUT, server.accept_adapter(&listener))
        .await
        .expect("accept adapter timed out")
        .expect("accept adapter failed");
    assert_eq!(language, "ruby");
    eprintln!("[portal-full-e2e] Ruby adapter connected");

    // --- Database setup + send configure to Ruby (same as Server::boot) ---
    server
        .setup_database()
        .await
        .expect("setup database");
    eprintln!("[portal-full-e2e] Database initialized, stdlib migrations sent to adapter");

    // Grab component references before server is moved into the background task.
    let player_store = server.player_store().expect("player_store after setup").clone();
    let repo_manager = server.repo_manager().expect("repo_manager after setup").clone();
    let workspace = server.workspace().expect("workspace after setup").clone();

    // --- Start Rust HTTP server (proxies portal routes to Ruby via Unix socket) ---
    let templates = Arc::new(init_templates().expect("init templates"));
    let state = AppState {
        player_store: Arc::clone(&player_store),
        repo_manager: Arc::clone(&repo_manager),
        workspace: Arc::clone(&workspace),
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
            eprintln!("[portal-full-e2e] HTTP server error: {}", e);
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

    // --- Wait for portal web server ---
    eprintln!(
        "[portal-full-e2e] Waiting for portal on http_port {}...",
        http_port
    );
    wait_for_portal(http_port, STARTUP_TIMEOUT).await;
    eprintln!("[portal-full-e2e] Portal ready");

    let base_url = format!("http://127.0.0.1:{}", http_port);

    // HTTP client with cookie jar (no auto-redirect so we can verify Location headers)
    let client = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    // ===================================================================
    // Phase 1: Register alice (builder by default)
    // ===================================================================
    eprintln!("[portal-full-e2e] Phase 1: Register alice (builder by default)");

    // POST /account/register (alice/secret123/Warrior)
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
    eprintln!("[portal-full-e2e]   alice registered (302 redirect)");

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
    eprintln!("[portal-full-e2e]   alice has builder role by default");

    // ===================================================================
    // Phase 2: Create repo via direct API call
    // ===================================================================
    eprintln!("[portal-full-e2e] Phase 2: Create repo alice/village");

    repo_manager
        .create_repo("alice", "village", true, None)
        .expect("create repo alice/village");
    workspace
        .checkout("alice", "village")
        .expect("checkout alice/village");
    eprintln!("[portal-full-e2e]   Repo alice/village created and checked out");

    // ===================================================================
    // Phase 3: GitApp API tests
    // ===================================================================
    eprintln!("[portal-full-e2e] Phase 3: GitApp API tests");

    // GET /git/api/repos -- JSON list of repos
    let resp = client
        .get(format!("{}/git/api/repos", base_url))
        .send()
        .await
        .expect("GET /git/api/repos");
    assert_eq!(resp.status(), 200, "GET /git/api/repos should return 200");
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("village"),
        "GET /git/api/repos should contain 'village'. Got:\n{}",
        &body[..body.len().min(500)]
    );
    eprintln!("[portal-full-e2e]   GET /git/api/repos: OK (contains 'village')");

    // GET /git/ -- dashboard page
    let resp = client
        .get(format!("{}/git/", base_url))
        .send()
        .await
        .expect("GET /git/");
    assert_eq!(resp.status(), 200, "GET /git/ should return 200");
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("village"),
        "GET /git/ dashboard should contain 'village'. Got:\n{}",
        &body[..body.len().min(500)]
    );
    eprintln!("[portal-full-e2e]   GET /git/: OK (contains 'village')");

    // ===================================================================
    // Phase 4: EditorApp tests
    // ===================================================================
    eprintln!("[portal-full-e2e] Phase 4: EditorApp tests");

    // GET /editor/ -- dashboard page
    let resp = client
        .get(format!("{}/editor/", base_url))
        .send()
        .await
        .expect("GET /editor/");
    assert_eq!(resp.status(), 200, "GET /editor/ should return 200");
    eprintln!("[portal-full-e2e]   GET /editor/: OK (200)");

    // POST /editor/api/pull -- pull repo
    let resp = client
        .post(format!("{}/editor/api/pull", base_url))
        .header("Content-Type", "application/json")
        .body(r#"{"repo": "alice/village"}"#)
        .send()
        .await
        .expect("POST /editor/api/pull");
    assert_eq!(
        resp.status(),
        200,
        "POST /editor/api/pull should return 200"
    );
    eprintln!("[portal-full-e2e]   POST /editor/api/pull: OK (200)");

    // GET /editor/api/files?repo=alice/village -- list files
    let resp = client
        .get(format!(
            "{}/editor/api/files?repo=alice/village",
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
    assert!(
        body.contains("entrance.rb"),
        "File listing should contain 'entrance.rb'. Got:\n{}",
        &body[..body.len().min(500)]
    );
    eprintln!("[portal-full-e2e]   GET /editor/api/files: OK (contains 'entrance.rb')");

    // GET /editor/api/files/rooms/entrance.rb?repo=alice/village -- read file
    let resp = client
        .get(format!(
            "{}/editor/api/files/rooms/entrance.rb?repo=alice/village",
            base_url
        ))
        .send()
        .await
        .expect("GET /editor/api/files/rooms/entrance.rb");
    assert_eq!(
        resp.status(),
        200,
        "GET entrance.rb should return 200"
    );
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("class Entrance"),
        "entrance.rb should contain 'class Entrance'. Got:\n{}",
        &body[..body.len().min(500)]
    );
    eprintln!("[portal-full-e2e]   GET entrance.rb: OK (contains 'class Entrance')");

    // PUT /editor/api/files/rooms/entrance.rb?repo=alice/village -- update file
    let resp = client
        .put(format!(
            "{}/editor/api/files/rooms/entrance.rb?repo=alice/village",
            base_url
        ))
        .header("Content-Type", "application/json")
        .body(r#"{"content": "class Entrance < Room\n  title \"Modified\"\nend\n"}"#)
        .send()
        .await
        .expect("PUT entrance.rb");
    assert_eq!(
        resp.status(),
        200,
        "PUT entrance.rb should return 200"
    );
    eprintln!("[portal-full-e2e]   PUT entrance.rb: OK (200)");

    // POST /editor/api/files/rooms/new_room.rb?repo=alice/village -- create file
    let resp = client
        .post(format!(
            "{}/editor/api/files/rooms/new_room.rb?repo=alice/village",
            base_url
        ))
        .header("Content-Type", "application/json")
        .body(r#"{"content": "class NewRoom < Room\nend\n"}"#)
        .send()
        .await
        .expect("POST new_room.rb");
    assert_eq!(
        resp.status(),
        201,
        "POST new_room.rb should return 201"
    );
    eprintln!("[portal-full-e2e]   POST new_room.rb: OK (201)");

    // DELETE /editor/api/files/rooms/new_room.rb?repo=alice/village -- delete file
    let resp = client
        .delete(format!(
            "{}/editor/api/files/rooms/new_room.rb?repo=alice/village",
            base_url
        ))
        .send()
        .await
        .expect("DELETE new_room.rb");
    assert_eq!(
        resp.status(),
        200,
        "DELETE new_room.rb should return 200"
    );
    eprintln!("[portal-full-e2e]   DELETE new_room.rb: OK (200)");

    // GET /editor/api/files/rooms/new_room.rb?repo=alice/village -- verify deleted
    let resp = client
        .get(format!(
            "{}/editor/api/files/rooms/new_room.rb?repo=alice/village",
            base_url
        ))
        .send()
        .await
        .expect("GET new_room.rb after delete");
    assert_eq!(
        resp.status(),
        404,
        "GET new_room.rb after delete should return 404"
    );
    eprintln!("[portal-full-e2e]   GET new_room.rb after delete: OK (404)");

    // ===================================================================
    // Phase 5: Access gating tests
    // ===================================================================
    eprintln!("[portal-full-e2e] Phase 5: Access gating tests");

    // --- Unauthenticated client ---
    let anon_client = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    // GET /git/ with no session -- should redirect to /account/login
    let resp = anon_client
        .get(format!("{}/git/", base_url))
        .send()
        .await
        .expect("GET /git/ unauthenticated");
    assert_eq!(
        resp.status(),
        302,
        "GET /git/ unauthenticated should redirect (302)"
    );
    let location = resp
        .headers()
        .get("location")
        .expect("/git/ redirect should have Location header")
        .to_str()
        .unwrap();
    assert!(
        location.ends_with("/account/login"),
        "GET /git/ unauthenticated should redirect to /account/login, got: {}",
        location
    );
    eprintln!("[portal-full-e2e]   GET /git/ unauthenticated: OK (302 -> /account/login)");

    // GET /editor/ with no session -- should redirect to /account/login
    let resp = anon_client
        .get(format!("{}/editor/", base_url))
        .send()
        .await
        .expect("GET /editor/ unauthenticated");
    assert_eq!(
        resp.status(),
        302,
        "GET /editor/ unauthenticated should redirect (302)"
    );
    let location = resp
        .headers()
        .get("location")
        .expect("/editor/ redirect should have Location header")
        .to_str()
        .unwrap();
    assert!(
        location.ends_with("/account/login"),
        "GET /editor/ unauthenticated should redirect to /account/login, got: {}",
        location
    );
    eprintln!("[portal-full-e2e]   GET /editor/ unauthenticated: OK (302 -> /account/login)");

    // --- Register bob then demote to player role for access-gating test ---
    let bob_client = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    let resp = bob_client
        .post(format!("{}/account/register", base_url))
        .form(&[
            ("username", "bob"),
            ("password", "password123"),
            ("character", "Rogue"),
        ])
        .send()
        .await
        .expect("POST register bob");
    assert_eq!(resp.status(), 302, "register bob should redirect");

    // Demote bob to player role via direct PlayerStore call
    player_store
        .set_role("bob", "player")
        .await
        .expect("demote bob to player");

    // Log bob in so session picks up the player role
    let resp = bob_client
        .post(format!("{}/account/login", base_url))
        .form(&[("username", "bob"), ("password", "password123")])
        .send()
        .await
        .expect("POST login bob");
    assert_eq!(resp.status(), 302, "login bob should redirect");

    // Follow redirect to establish bob's session
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
        let _ = bob_client
            .get(&redirect_url)
            .send()
            .await
            .expect("GET redirect after bob login");
    }
    eprintln!("[portal-full-e2e]   bob registered and demoted to player");

    // GET /git/ as bob (player, not builder) -- require_builder! returns 403
    let resp = bob_client
        .get(format!("{}/git/", base_url))
        .send()
        .await
        .expect("GET /git/ as bob");
    assert_eq!(
        resp.status(),
        403,
        "GET /git/ as bob (player) should return 403"
    );
    eprintln!("[portal-full-e2e]   GET /git/ as bob (player): OK (403)");

    // GET /editor/ as bob (player, not builder) -- require_builder! returns 403
    let resp = bob_client
        .get(format!("{}/editor/", base_url))
        .send()
        .await
        .expect("GET /editor/ as bob");
    assert_eq!(
        resp.status(),
        403,
        "GET /editor/ as bob (player) should return 403"
    );
    eprintln!("[portal-full-e2e]   GET /editor/ as bob (player): OK (403)");

    // ===================================================================
    // Cleanup
    // ===================================================================
    eprintln!("[portal-full-e2e] Cleaning up...");
    bg_handle.abort();
    let _ = bg_handle.await;
    // _ruby_process, _container, temp_dir cleaned up on drop
    eprintln!("[portal-full-e2e] Portal full E2E test passed!");
}
