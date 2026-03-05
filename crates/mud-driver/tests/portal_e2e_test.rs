//! Full-stack portal E2E test: testcontainers PostgreSQL + Rust driver Server
//! + real Ruby adapter process + Rust HTTP proxy + HTTP client.
//!
//! Tests the complete register → login flow through the actual web interface:
//! HTTP requests hit the Rust HTTP server, which proxies portal routes to the
//! Ruby adapter via a Unix domain socket. The Ruby adapter communicates with
//! the Rust driver via MOP for account operations backed by PostgreSQL.
//!
//! Run with: `cargo test -p mud-driver --test portal_e2e_test -- --ignored --nocapture`
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
    fn spawn(socket_path: &Path, web_socket_path: &Path) -> Self {
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

// ===========================================================================
// Tests
// ===========================================================================

/// Full portal E2E test: register an account through the web UI, log out,
/// log back in, verify wrong credentials are rejected.
///
/// This test covers the exact bug where the Ruby portal's MOP client was nil
/// due to class-level instance variable inheritance, causing all account
/// operations to silently fail.
#[tokio::test]
#[ignore] // Requires Docker + Ruby; run with: cargo test -- --ignored
async fn portal_register_and_login_e2e() {
    if !prerequisites_met() {
        return;
    }

    eprintln!("[portal-e2e] Starting full-stack portal E2E test");

    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let socket_path = temp_dir.path().join("portal-e2e.sock");
    let web_socket_path = temp_dir.path().join("portal-web.sock");
    let http_port = find_free_port();

    // --- Temp directories for world and repos ---
    let repos_path = temp_dir.path().join("repos");
    std::fs::create_dir_all(&repos_path).expect("create repos dir");
    let world_path = temp_dir.path().join("world");
    std::fs::create_dir_all(&world_path).expect("create world dir");

    // --- PostgreSQL via testcontainers ---
    eprintln!("[portal-e2e] Starting PostgreSQL container...");
    let container = Postgres::default()
        .start()
        .await
        .expect("start PostgreSQL container");

    let host_port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("get PostgreSQL port");

    eprintln!("[portal-e2e] PostgreSQL ready on port {}", host_port);

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
    eprintln!("[portal-e2e] Adapter manager started");

    // --- Spawn Ruby adapter ---
    let _ruby_process = RubyAdapterProcess::spawn(&socket_path, &web_socket_path);
    eprintln!("[portal-e2e] Ruby adapter spawned (web socket {:?})", web_socket_path);

    // --- Accept adapter connection (handshake) ---
    let language = timeout(STARTUP_TIMEOUT, server.accept_adapter(&listener))
        .await
        .expect("accept adapter timed out")
        .expect("accept adapter failed");
    assert_eq!(language, "ruby");
    eprintln!("[portal-e2e] Ruby adapter connected");

    // --- Database setup + send configure to Ruby (same as Server::boot) ---
    server
        .setup_database()
        .await
        .expect("setup database");
    eprintln!("[portal-e2e] Database initialized, stdlib migrations sent to adapter");

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
            eprintln!("[portal-e2e] HTTP server error: {}", e);
        }
    });

    // --- Run server MOP message loop in background ---
    // The portal web server sends MOP requests (player_create, player_authenticate,
    // etc.) which need to be processed by the Rust server concurrently.
    let bg_handle = tokio::spawn(async move {
        loop {
            match server.recv_adapter_message().await {
                Some(msg) => server.handle_adapter_message(msg).await,
                None => break,
            }
        }
    });

    // --- Wait for portal to be ready (Ruby via Rust proxy) ---
    eprintln!("[portal-e2e] Waiting for portal on Rust HTTP port {}...", http_port);
    wait_for_portal(http_port, STARTUP_TIMEOUT).await;
    eprintln!("[portal-e2e] Portal ready");

    let base_url = format!("http://127.0.0.1:{}", http_port);

    // HTTP client with cookie jar (no auto-redirect so we can verify Location headers)
    let client = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    // -----------------------------------------------------------------------
    // Step 1: GET /account/register — render register form
    // -----------------------------------------------------------------------
    eprintln!("[portal-e2e] Step 1: GET /account/register");
    let resp = client
        .get(format!("{}/account/register", base_url))
        .send()
        .await
        .expect("GET register");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("register") || body.contains("Register"),
        "Register page should contain 'Register'. Got:\n{}",
        &body[..body.len().min(500)]
    );

    // -----------------------------------------------------------------------
    // Step 2: POST /account/register — create account "alice"
    // -----------------------------------------------------------------------
    eprintln!("[portal-e2e] Step 2: POST /account/register (alice/secret123/Warrior)");
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
    let register_status = resp.status();
    let register_location = resp
        .headers()
        .get("location")
        .map(|v| v.to_str().unwrap_or("").to_string());
    assert_eq!(
        register_status,
        302,
        "register should redirect. Location: {:?}",
        register_location
    );

    // -----------------------------------------------------------------------
    // Step 3: GET /account/characters — verify logged in after register
    // -----------------------------------------------------------------------
    eprintln!("[portal-e2e] Step 3: GET /account/characters (should be logged in)");
    let resp = client
        .get(format!("{}/account/characters", base_url))
        .send()
        .await
        .expect("GET characters");
    assert_eq!(resp.status(), 200, "should be logged in and see characters page");
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("Warrior"),
        "Characters page should show 'Warrior'. Got:\n{}",
        &body[..body.len().min(500)]
    );
    eprintln!("[portal-e2e]   Characters page shows 'Warrior'");

    // -----------------------------------------------------------------------
    // Step 4: POST /account/logout
    // -----------------------------------------------------------------------
    eprintln!("[portal-e2e] Step 4: POST /account/logout");
    let resp = client
        .post(format!("{}/account/logout", base_url))
        .send()
        .await
        .expect("POST logout");
    assert_eq!(resp.status(), 302, "logout should redirect");
    let location = resp
        .headers()
        .get("location")
        .expect("logout should have Location header")
        .to_str()
        .unwrap();
    assert!(
        location.ends_with("/account/login"),
        "logout should redirect to /account/login, got: {}",
        location
    );

    // -----------------------------------------------------------------------
    // Step 5: GET /account/characters after logout — should redirect to login
    // -----------------------------------------------------------------------
    eprintln!("[portal-e2e] Step 5: GET /account/characters (after logout, should redirect)");
    let resp = client
        .get(format!("{}/account/characters", base_url))
        .send()
        .await
        .expect("GET characters after logout");
    assert_eq!(
        resp.status(),
        302,
        "should redirect to login after logout"
    );
    let location = resp
        .headers()
        .get("location")
        .expect("should have Location header")
        .to_str()
        .unwrap();
    assert!(
        location.ends_with("/account/login"),
        "should redirect to login, got: {}",
        location
    );

    // -----------------------------------------------------------------------
    // Step 6: POST /account/login — correct credentials
    // -----------------------------------------------------------------------
    eprintln!("[portal-e2e] Step 6: POST /account/login (correct credentials)");
    let resp = client
        .post(format!("{}/account/login", base_url))
        .form(&[("username", "alice"), ("password", "secret123")])
        .send()
        .await
        .expect("POST login");
    assert_eq!(resp.status(), 302, "login should redirect on success");
    let location = resp
        .headers()
        .get("location")
        .expect("login should have Location header")
        .to_str()
        .unwrap();
    assert!(
        location.ends_with("/account/characters"),
        "login should redirect to characters, got: {}",
        location
    );

    // -----------------------------------------------------------------------
    // Step 7: GET /account/characters — verify logged in again
    // -----------------------------------------------------------------------
    eprintln!("[portal-e2e] Step 7: GET /account/characters (logged in after login)");
    let resp = client
        .get(format!("{}/account/characters", base_url))
        .send()
        .await
        .expect("GET characters after login");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("Warrior"),
        "Characters page should show 'Warrior' after login. Got:\n{}",
        &body[..body.len().min(500)]
    );

    // -----------------------------------------------------------------------
    // Step 8: POST /account/login — wrong password (fresh client, no session)
    // -----------------------------------------------------------------------
    eprintln!("[portal-e2e] Step 8: POST /account/login (wrong password)");
    let bad_client = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let resp = bad_client
        .post(format!("{}/account/login", base_url))
        .form(&[("username", "alice"), ("password", "wrongpassword")])
        .send()
        .await
        .expect("POST login with wrong password");
    assert_eq!(
        resp.status(),
        200,
        "failed login should stay on login page"
    );
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("Invalid credentials"),
        "Wrong password should show 'Invalid credentials'. Got:\n{}",
        &body[..body.len().min(500)]
    );

    // -----------------------------------------------------------------------
    // Step 9: POST /account/login — nonexistent user
    // -----------------------------------------------------------------------
    eprintln!("[portal-e2e] Step 9: POST /account/login (nonexistent user)");
    let resp = bad_client
        .post(format!("{}/account/login", base_url))
        .form(&[("username", "nonexistent"), ("password", "whatever")])
        .send()
        .await
        .expect("POST login for nonexistent user");
    assert_eq!(
        resp.status(),
        200,
        "nonexistent user login should stay on login page"
    );
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("Invalid credentials"),
        "Nonexistent user should show 'Invalid credentials'. Got:\n{}",
        &body[..body.len().min(500)]
    );

    // -----------------------------------------------------------------------
    // Step 10: POST /account/register — duplicate username
    // -----------------------------------------------------------------------
    eprintln!("[portal-e2e] Step 10: POST /account/register (duplicate username)");
    let dup_client = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let resp = dup_client
        .post(format!("{}/account/register", base_url))
        .form(&[
            ("username", "alice"),
            ("password", "other123"),
            ("character", "Mage"),
        ])
        .send()
        .await
        .expect("POST register duplicate");
    assert_eq!(
        resp.status(),
        200,
        "duplicate register should stay on register page"
    );
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("already taken") || body.contains("Username already taken"),
        "Duplicate register should show 'Username already taken'. Got:\n{}",
        &body[..body.len().min(500)]
    );

    // -----------------------------------------------------------------------
    // Cleanup
    // -----------------------------------------------------------------------
    eprintln!("[portal-e2e] Cleaning up...");
    bg_handle.abort();
    let _ = bg_handle.await;
    // _ruby_process, _container, temp_dir cleaned up on drop
    eprintln!("[portal-e2e] Portal E2E test passed (10 steps)!");
}
