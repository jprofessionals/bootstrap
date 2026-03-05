//! E2E test for git operations in the portal GitApp.
//!
//! Tests the full flow: register (auto-creates area) → list repos →
//! list branches → view log → view diff → edit file → commit → pull →
//! create branch → verify new branch.
//!
//! HTTP requests hit the Rust HTTP server, which proxies portal routes to the
//! Ruby adapter via a Unix domain socket. The Ruby adapter communicates with
//! the Rust driver via MOP for account operations backed by PostgreSQL.
//!
//! Run with: `cargo test -p mud-driver --test portal_git_ops_e2e_test -- --ignored --nocapture`
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

/// Full git operations E2E test:
///
/// Phase 0: Setup (testcontainers PG + Server + Ruby adapter)
/// Phase 1: Register alice (builder, auto-creates area alice/alice)
/// Phase 2: List repos — verify alice/alice exists
/// Phase 3: List branches — verify main and develop
/// Phase 4: View commit log — verify seed commit
/// Phase 5: View diff — verify clean working tree
/// Phase 6: Edit file via editor, then commit via git API
/// Phase 7: View log again — verify new commit appears
/// Phase 8: Pull — verify pull succeeds
/// Phase 9: Create branch — verify new branch appears
/// Phase 10: Start play session — verify room from committed area
/// Phase 11: Edit room + commit — verify area reload updates play session
/// Phase 12: Register bob — verify bob cannot access alice's editor files (403)
/// Phase 13: Verify dev changes visible in editor before merge to main
/// Phase 14: Access repo without workspace — verify graceful error handling
/// Phase 15: Editor pull auto-creates workspace when missing
#[tokio::test]
#[ignore] // Requires Docker + Ruby; run with: cargo test -- --ignored
async fn git_operations_e2e() {
    if !prerequisites_met() {
        return;
    }

    eprintln!("[git-ops-e2e] Starting git operations E2E test");

    // ===================================================================
    // Phase 0: Setup
    // ===================================================================

    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let socket_path = temp_dir.path().join("git-ops-e2e.sock");
    let web_socket_path = temp_dir.path().join("portal-web.sock");
    let http_port = find_free_port();

    eprintln!("[git-ops-e2e] Starting PostgreSQL container...");
    let container = Postgres::default()
        .start()
        .await
        .expect("start PostgreSQL container");

    let host_port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("get PostgreSQL port");

    eprintln!("[git-ops-e2e] PostgreSQL ready on port {}", host_port);

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
    eprintln!("[git-ops-e2e] Ruby adapter spawned (web socket {:?})", web_socket_path);

    let language = timeout(STARTUP_TIMEOUT, server.accept_adapter(&listener))
        .await
        .expect("accept adapter timed out")
        .expect("accept adapter failed");
    assert_eq!(language, "ruby");

    server
        .setup_database()
        .await
        .expect("setup database");
    eprintln!("[git-ops-e2e] Database initialized");

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
            eprintln!("[git-ops-e2e] HTTP server error: {}", e);
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

    wait_for_portal(http_port, STARTUP_TIMEOUT).await;
    eprintln!("[git-ops-e2e] Portal ready (Rust HTTP proxy + Ruby backend)");

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
    eprintln!("[git-ops-e2e] Phase 1: Register alice");

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
    eprintln!("[git-ops-e2e]   alice registered");

    // ===================================================================
    // Phase 2: List repos — verify alice/alice exists
    // ===================================================================
    eprintln!("[git-ops-e2e] Phase 2: List repos");

    let resp = client
        .get(format!("{}/git/api/repos", base_url))
        .send()
        .await
        .expect("GET /git/api/repos");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("alice/alice"),
        "repos should contain 'alice/alice'. Got: {}",
        &body[..body.len().min(500)]
    );
    eprintln!("[git-ops-e2e]   repos: {}", body.trim());

    // ===================================================================
    // Phase 3: List branches — verify main and develop
    // ===================================================================
    eprintln!("[git-ops-e2e] Phase 3: List branches");

    let resp = client
        .get(format!("{}/git/api/repos/alice/alice/branches", base_url))
        .send()
        .await
        .expect("GET branches");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("main"),
        "branches should contain 'main'. Got: {}",
        &body[..body.len().min(500)]
    );
    assert!(
        body.contains("develop"),
        "branches should contain 'develop'. Got: {}",
        &body[..body.len().min(500)]
    );
    eprintln!("[git-ops-e2e]   branches: {}", body.trim());

    // ===================================================================
    // Phase 4: View commit log — verify seed commit
    // ===================================================================
    eprintln!("[git-ops-e2e] Phase 4: View commit log");

    let resp = client
        .get(format!(
            "{}/git/api/repos/alice/alice/log?branch=develop",
            base_url
        ))
        .send()
        .await
        .expect("GET log");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("Initial area template"),
        "log should contain seed commit message. Got: {}",
        &body[..body.len().min(500)]
    );
    eprintln!("[git-ops-e2e]   log contains seed commit");

    // ===================================================================
    // Phase 5: View diff — verify clean working tree
    // ===================================================================
    eprintln!("[git-ops-e2e] Phase 5: View diff (clean tree)");

    let resp = client
        .get(format!(
            "{}/git/api/repos/alice/alice/diff?branch=develop",
            base_url
        ))
        .send()
        .await
        .expect("GET diff");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // A freshly checked out repo should have no changes
    assert!(
        body.contains("\"changes\":[]"),
        "diff should be empty for clean tree. Got: {}",
        &body[..body.len().min(500)]
    );
    eprintln!("[git-ops-e2e]   diff is clean");

    // ===================================================================
    // Phase 6: Edit file via editor API, then commit via git API
    // ===================================================================
    eprintln!("[git-ops-e2e] Phase 6: Edit file + commit");

    // Update entrance.rb via the editor API
    let resp = client
        .put(format!(
            "{}/editor/api/files/rooms/entrance.rb?repo=alice/alice",
            base_url
        ))
        .header("Content-Type", "application/json")
        .body(r#"{"content": "class Entrance < Room\n  title \"Modified Entrance\"\n  description \"Changed by test.\"\nend\n"}"#)
        .send()
        .await
        .expect("PUT entrance.rb");
    assert_eq!(resp.status(), 200, "PUT entrance.rb should succeed");
    eprintln!("[git-ops-e2e]   entrance.rb updated");

    // Verify diff now shows changes
    let resp = client
        .get(format!(
            "{}/git/api/repos/alice/alice/diff?branch=develop",
            base_url
        ))
        .send()
        .await
        .expect("GET diff after edit");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("entrance.rb"),
        "diff should show entrance.rb changed. Got: {}",
        &body[..body.len().min(500)]
    );
    eprintln!("[git-ops-e2e]   diff shows entrance.rb modified");

    // Commit the changes via git API
    let resp = client
        .post(format!(
            "{}/git/api/repos/alice/alice/commit",
            base_url
        ))
        .header("Content-Type", "application/json")
        .body(r#"{"message": "Update entrance room"}"#)
        .send()
        .await
        .expect("POST commit");
    assert_eq!(resp.status(), 200, "commit should succeed");
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("committed"),
        "commit response should confirm. Got: {}",
        &body[..body.len().min(500)]
    );
    eprintln!("[git-ops-e2e]   commit succeeded");

    // ===================================================================
    // Phase 7: View log again — verify new commit
    // ===================================================================
    eprintln!("[git-ops-e2e] Phase 7: Verify commit in log");

    let resp = client
        .get(format!(
            "{}/git/api/repos/alice/alice/log?branch=develop",
            base_url
        ))
        .send()
        .await
        .expect("GET log after commit");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("Update entrance room"),
        "log should contain new commit message. Got: {}",
        &body[..body.len().min(500)]
    );
    assert!(
        body.contains("Initial area template"),
        "log should still contain seed commit. Got: {}",
        &body[..body.len().min(500)]
    );
    eprintln!("[git-ops-e2e]   log shows both commits");

    // ===================================================================
    // Phase 8: Pull — verify no error
    // ===================================================================
    eprintln!("[git-ops-e2e] Phase 8: Pull");

    let resp = client
        .post(format!("{}/editor/api/pull", base_url))
        .header("Content-Type", "application/json")
        .body(r#"{"repo": "alice/alice"}"#)
        .send()
        .await
        .expect("POST pull");
    assert_eq!(resp.status(), 200, "pull should succeed");
    eprintln!("[git-ops-e2e]   pull succeeded");

    // ===================================================================
    // Phase 9: Create branch — verify it appears
    // ===================================================================
    eprintln!("[git-ops-e2e] Phase 9: Create branch");

    let resp = client
        .post(format!(
            "{}/git/api/repos/alice/alice/branches",
            base_url
        ))
        .header("Content-Type", "application/json")
        .body(r#"{"name": "feature_quest"}"#)
        .send()
        .await
        .expect("POST create branch");
    assert_eq!(resp.status(), 201, "create branch should return 201");
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("feature_quest"),
        "response should confirm branch name. Got: {}",
        &body[..body.len().min(500)]
    );
    eprintln!("[git-ops-e2e]   branch feature_quest created");

    // Verify the new branch appears in the list
    let resp = client
        .get(format!("{}/git/api/repos/alice/alice/branches", base_url))
        .send()
        .await
        .expect("GET branches after create");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("feature_quest"),
        "branches should contain 'feature_quest'. Got: {}",
        &body[..body.len().min(500)]
    );
    assert!(
        body.contains("main"),
        "branches should still contain 'main'. Got: {}",
        &body[..body.len().min(500)]
    );
    assert!(
        body.contains("develop"),
        "branches should still contain 'develop'. Got: {}",
        &body[..body.len().min(500)]
    );
    eprintln!("[git-ops-e2e]   branches list includes feature_quest");

    // ===================================================================
    // Phase 10: Start play session — verify room from committed area
    // ===================================================================
    // The commit in Phase 6 triggered area_reload, which loaded the area
    // from the @dev workspace. The entrance room should show "Modified Entrance".
    eprintln!("[git-ops-e2e] Phase 10: Start play session");

    let resp = client
        .post(format!("{}/play/start", base_url))
        .send()
        .await
        .expect("POST /play/start");
    assert_eq!(resp.status(), 200, "POST /play/start should return 200");
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("Modified Entrance"),
        "play session should show 'Modified Entrance' from Phase 6 commit. Got:\n{}",
        &body[..body.len().min(500)]
    );
    eprintln!("[git-ops-e2e]   play session shows 'Modified Entrance'");

    // ===================================================================
    // Phase 11: Edit room + commit — verify area reload updates play session
    // ===================================================================
    eprintln!("[git-ops-e2e] Phase 11: Edit + commit + verify reload in play");

    // Edit entrance.rb to a new description
    let resp = client
        .put(format!(
            "{}/editor/api/files/rooms/entrance.rb?repo=alice/alice",
            base_url
        ))
        .header("Content-Type", "application/json")
        .body(r#"{"content": "class Entrance < Room\n  title \"The Grand Foyer\"\n  description \"A magnificent hall with marble columns.\"\nend\n"}"#)
        .send()
        .await
        .expect("PUT entrance.rb (phase 11)");
    assert_eq!(resp.status(), 200, "PUT entrance.rb should succeed");

    // Commit (triggers area reload)
    let resp = client
        .post(format!(
            "{}/git/api/repos/alice/alice/commit",
            base_url
        ))
        .header("Content-Type", "application/json")
        .body(r#"{"message": "Redesign entrance as grand foyer"}"#)
        .send()
        .await
        .expect("POST commit (phase 11)");
    assert_eq!(resp.status(), 200, "commit should succeed");
    eprintln!("[git-ops-e2e]   committed entrance.rb update");

    // Execute "look" command — should reflect the reload
    let resp = client
        .post(format!("{}/play/command", base_url))
        .header("Content-Type", "application/json")
        .body(r#"{"input": "look"}"#)
        .send()
        .await
        .expect("POST /play/command (look)");
    assert_eq!(resp.status(), 200, "look command should return 200");
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("The Grand Foyer"),
        "look should show 'The Grand Foyer' after reload. Got:\n{}",
        &body[..body.len().min(500)]
    );
    eprintln!("[git-ops-e2e]   play session shows 'The Grand Foyer' after reload");

    // ===================================================================
    // Phase 12: Register bob — verify bob cannot access alice's editor files
    // ===================================================================
    eprintln!("[git-ops-e2e] Phase 12: Access restriction (bob vs alice's repo)");

    // Use a separate HTTP client for bob (separate cookie jar)
    let bob_client = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    // Register bob
    let resp = bob_client
        .post(format!("{}/account/register", base_url))
        .form(&[
            ("username", "bob"),
            ("password", "bobpass123"),
            ("character", "Wizard"),
        ])
        .send()
        .await
        .expect("POST register bob");
    assert_eq!(resp.status(), 302, "bob register should redirect");

    // Follow redirect to establish session
    if let Some(loc) = resp.headers().get("location") {
        let loc = loc.to_str().unwrap();
        let url = if loc.starts_with('/') {
            format!("{}{}", base_url, loc)
        } else {
            loc.to_string()
        };
        bob_client.get(&url).send().await.expect("follow redirect bob");
    }
    eprintln!("[git-ops-e2e]   bob registered");

    // Bob should be able to list his own repos (bob/bob)
    let resp = bob_client
        .get(format!("{}/git/api/repos", base_url))
        .send()
        .await
        .expect("bob GET /git/api/repos");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("bob/bob"),
        "bob's repos should contain 'bob/bob'. Got: {}",
        &body[..body.len().min(500)]
    );
    eprintln!("[git-ops-e2e]   bob can see his own repo");

    // Bob should NOT be able to access alice's editor files (403)
    let resp = bob_client
        .get(format!(
            "{}/editor/api/files?repo=alice/alice",
            base_url
        ))
        .send()
        .await
        .expect("bob GET alice's editor files");
    assert_eq!(
        resp.status(),
        403,
        "bob should get 403 accessing alice's editor files. Got: {}",
        resp.status()
    );
    eprintln!("[git-ops-e2e]   bob gets 403 on alice's editor files");

    // Bob should NOT be able to access alice's git API (403)
    let resp = bob_client
        .get(format!(
            "{}/git/api/repos/alice/alice/branches",
            base_url
        ))
        .send()
        .await
        .expect("bob GET alice's git branches");
    assert_eq!(
        resp.status(),
        403,
        "bob should get 403 accessing alice's git API. Got: {}",
        resp.status()
    );
    eprintln!("[git-ops-e2e]   bob gets 403 on alice's git API");

    // Bob CAN access his own editor files
    let resp = bob_client
        .get(format!(
            "{}/editor/api/files?repo=bob/bob",
            base_url
        ))
        .send()
        .await
        .expect("bob GET own editor files");
    assert_eq!(
        resp.status(),
        200,
        "bob should access his own editor files. Got: {}",
        resp.status()
    );
    eprintln!("[git-ops-e2e]   bob can access his own editor files");

    // ===================================================================
    // Phase 13: Verify dev changes visible in editor before merge to main
    // ===================================================================
    // Alice edited entrance.rb on the develop branch (Phase 11).
    // The @dev workspace should show the new content.
    // The main workspace should NOT have the Phase 11 changes yet.
    eprintln!("[git-ops-e2e] Phase 13: Dev vs main content divergence");

    // Read entrance.rb from alice's editor (resolves to @dev workspace)
    let resp = client
        .get(format!(
            "{}/editor/api/files/rooms/entrance.rb?repo=alice/alice",
            base_url
        ))
        .send()
        .await
        .expect("GET entrance.rb from dev workspace");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("The Grand Foyer"),
        "dev workspace should contain 'The Grand Foyer'. Got:\n{}",
        &body[..body.len().min(500)]
    );
    eprintln!("[git-ops-e2e]   dev workspace shows 'The Grand Foyer'");

    // The develop branch log should show the Phase 11 commit
    let resp = client
        .get(format!(
            "{}/git/api/repos/alice/alice/log?branch=develop",
            base_url
        ))
        .send()
        .await
        .expect("GET log develop branch");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("Redesign entrance as grand foyer"),
        "develop branch should contain Phase 11 commit. Got:\n{}",
        &body[..body.len().min(500)]
    );
    eprintln!("[git-ops-e2e]   develop log shows Phase 11 commit");

    // The main branch log should NOT yet have the Phase 11 commit
    let resp = client
        .get(format!(
            "{}/git/api/repos/alice/alice/log?branch=main",
            base_url
        ))
        .send()
        .await
        .expect("GET log main branch");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        !body.contains("Redesign entrance as grand foyer"),
        "main branch should NOT contain Phase 11 commit before merge. Got:\n{}",
        &body[..body.len().min(500)]
    );
    eprintln!("[git-ops-e2e]   main branch does NOT have Phase 11 commit (not merged yet)");

    // ===================================================================
    // Phase 14: Access repo without workspace — graceful error handling
    // ===================================================================
    // A repo may exist (bare repo + ACL) without a checked-out workspace.
    // The git dashboard endpoints should return 200 with empty data
    // instead of crashing with 500.
    eprintln!("[git-ops-e2e] Phase 14: Repo without workspace (graceful errors)");

    // Create a bare repo for alice without checking out a workspace.
    // We do this by creating the bare git repo directory directly.
    let bare_repo_path = repos_path.join("alice").join("orphan.git");
    std::fs::create_dir_all(&bare_repo_path).expect("create bare repo dir");
    git2::Repository::init_bare(&bare_repo_path).expect("init bare repo");
    // Write ACL so alice has access
    let acl_path = repos_path.join("alice").join("orphan.git.acl.yml");
    std::fs::write(&acl_path, "owner: alice\ncollaborators: {}\n")
        .expect("write acl");

    // GET log — should return 200 with empty commits + error message
    let resp = client
        .get(format!(
            "{}/git/api/repos/alice/orphan/log",
            base_url
        ))
        .send()
        .await
        .expect("GET log for orphan repo");
    assert_eq!(
        resp.status(),
        200,
        "log should return 200 for repo without workspace, not 500"
    );
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("\"commits\":[]"),
        "log should return empty commits. Got: {}",
        &body[..body.len().min(500)]
    );
    eprintln!("[git-ops-e2e]   GET log for orphan repo: OK (200 with empty commits)");

    // GET diff — should return 200 with empty changes
    let resp = client
        .get(format!(
            "{}/git/api/repos/alice/orphan/diff",
            base_url
        ))
        .send()
        .await
        .expect("GET diff for orphan repo");
    assert_eq!(
        resp.status(),
        200,
        "diff should return 200 for repo without workspace, not 500"
    );
    eprintln!("[git-ops-e2e]   GET diff for orphan repo: OK (200)");

    // GET branches — should return 200 with empty branches
    let resp = client
        .get(format!(
            "{}/git/api/repos/alice/orphan/branches",
            base_url
        ))
        .send()
        .await
        .expect("GET branches for orphan repo");
    assert_eq!(
        resp.status(),
        200,
        "branches should return 200 for repo without workspace, not 500"
    );
    eprintln!("[git-ops-e2e]   GET branches for orphan repo: OK (200)");

    // GET merge-requests — should return 200 with empty list
    let resp = client
        .get(format!(
            "{}/git/api/repos/alice/orphan/merge-requests",
            base_url
        ))
        .send()
        .await
        .expect("GET merge-requests for orphan repo");
    assert_eq!(
        resp.status(),
        200,
        "merge-requests should return 200 for repo without workspace, not 500"
    );
    eprintln!("[git-ops-e2e]   GET merge-requests for orphan repo: OK (200)");

    // ===================================================================
    // Phase 15: Editor pull auto-creates workspace when missing
    // ===================================================================
    // Bob has a registered area (bob/bob) with working dirs from Phase 12.
    // Remove the working dirs, then verify that editor pull re-creates them.
    eprintln!("[git-ops-e2e] Phase 15: Editor pull auto-creates workspace");

    let bob_prod = world_path.join("bob").join("bob");
    let bob_dev = world_path.join("bob").join("bob@dev");
    assert!(bob_prod.exists(), "bob's production workspace should exist before removal");
    assert!(bob_dev.exists(), "bob's dev workspace should exist before removal");

    std::fs::remove_dir_all(&bob_prod).expect("remove bob prod workspace");
    std::fs::remove_dir_all(&bob_dev).expect("remove bob dev workspace");
    assert!(!bob_prod.exists(), "bob's production workspace should be gone");
    assert!(!bob_dev.exists(), "bob's dev workspace should be gone");
    eprintln!("[git-ops-e2e]   removed bob's working directories");

    // Editor pull should auto-checkout the working dirs
    let resp = bob_client
        .post(format!("{}/editor/api/pull", base_url))
        .header("Content-Type", "application/json")
        .body(r#"{"repo": "bob/bob"}"#)
        .send()
        .await
        .expect("POST pull for bob (no workspace)");
    assert_eq!(
        resp.status(),
        200,
        "editor pull should auto-create workspace and succeed"
    );
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("files"),
        "pull response should contain files listing. Got: {}",
        &body[..body.len().min(500)]
    );
    eprintln!("[git-ops-e2e]   editor pull succeeded (auto-created workspace)");

    // Verify bob can now list files through the editor
    let resp = bob_client
        .get(format!(
            "{}/editor/api/files?repo=bob/bob",
            base_url
        ))
        .send()
        .await
        .expect("GET bob's editor files after pull");
    assert_eq!(
        resp.status(),
        200,
        "bob should access editor files after auto-checkout pull"
    );
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("entrance.rb"),
        "bob's files should contain seeded entrance.rb. Got: {}",
        &body[..body.len().min(500)]
    );
    assert!(
        body.contains("mud_aliases.rb"),
        "bob's files should contain mud_aliases.rb. Got: {}",
        &body[..body.len().min(500)]
    );
    assert!(
        body.contains("mud_loader.rb"),
        "bob's files should contain mud_loader.rb. Got: {}",
        &body[..body.len().min(500)]
    );
    eprintln!("[git-ops-e2e]   bob's files accessible after auto-checkout pull");

    // Verify template content was properly substituted
    let resp = bob_client
        .get(format!(
            "{}/editor/api/files/mud_aliases.rb?repo=bob/bob",
            base_url
        ))
        .send()
        .await
        .expect("GET bob's mud_aliases.rb");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("Room = MUD::Stdlib::World::Room"),
        "mud_aliases.rb should contain Room alias. Got: {}",
        &body[..body.len().min(500)]
    );
    assert!(
        body.contains("Daemon = MUD::Stdlib::World::Daemon"),
        "mud_aliases.rb should contain Daemon alias. Got: {}",
        &body[..body.len().min(500)]
    );
    eprintln!("[git-ops-e2e]   bob's mud_aliases.rb has expected content");

    let resp = bob_client
        .get(format!(
            "{}/editor/api/files/rooms/entrance.rb?repo=bob/bob",
            base_url
        ))
        .send()
        .await
        .expect("GET bob's entrance.rb");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("Welcome to bob"),
        "entrance.rb should contain area name substitution. Got: {}",
        &body[..body.len().min(500)]
    );
    eprintln!("[git-ops-e2e]   bob's entrance.rb has area name substituted");

    // ===================================================================
    // Cleanup
    // ===================================================================
    eprintln!("[git-ops-e2e] Cleaning up...");
    bg_handle.abort();
    let _ = bg_handle.await;
    eprintln!("[git-ops-e2e] Git operations E2E test passed!");
}
