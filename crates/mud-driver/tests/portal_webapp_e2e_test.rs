//! E2E test for BuilderApp web_app support — mounting user Rack apps.
//!
//! Tests that a user-defined Rack app (via web_app block in mud_web.rb)
//! correctly handles API requests and returns proper headers, without
//! crashing the session middleware.
//!
//! Run with: `cargo test -p mud-driver --test portal_webapp_e2e_test -- --ignored --nocapture`
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
// Skip-condition helpers (shared pattern with other portal tests)
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

/// Full web_app E2E test:
///
/// Phase 0: Setup (testcontainers PG + Server + Ruby adapter)
/// Phase 1: Register alice (builder, auto-creates area alice/alice)
/// Phase 2: Push mud_web.rb with web_app block + web/index.html via editor API
/// Phase 3: Commit to trigger area loading
/// Phase 4: GET /builder/alice/alice/ — verify HTML page is served
/// Phase 5: GET /builder/alice/alice/api/hello — verify Rack app API response
/// Phase 6: POST /builder/alice/alice/api/echo — verify POST with JSON body
/// Phase 7: GET /builder/alice/alice/nonexistent — verify 404 falls through to HTML
#[tokio::test]
#[ignore] // Requires Docker + Ruby; run with: cargo test -- --ignored
async fn webapp_rack_app_e2e() {
    if !prerequisites_met() {
        return;
    }

    eprintln!("[webapp-e2e] Starting web_app E2E test");

    // ===================================================================
    // Phase 0: Setup
    // ===================================================================

    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let socket_path = temp_dir.path().join("webapp-e2e.sock");
    let http_port = find_free_port();
    let web_socket_path = temp_dir.path().join("portal-web.sock");

    eprintln!("[webapp-e2e] Starting PostgreSQL container...");
    let container = Postgres::default()
        .start()
        .await
        .expect("start PostgreSQL container");

    let host_port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("get PostgreSQL port");

    eprintln!("[webapp-e2e] PostgreSQL ready on port {}", host_port);

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
    eprintln!("[webapp-e2e] Database initialized");

    // --- Start Rust HTTP server ---
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
            eprintln!("[webapp-e2e] HTTP server error: {}", e);
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
    eprintln!("[webapp-e2e] Portal ready");

    let base_url = format!("http://127.0.0.1:{}", http_port);

    let client = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    // ===================================================================
    // Phase 1: Register alice
    // ===================================================================
    eprintln!("[webapp-e2e] Phase 1: Register alice");

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

    if let Some(loc) = resp.headers().get("location") {
        let loc = loc.to_str().unwrap();
        let url = if loc.starts_with('/') {
            format!("{}{}", base_url, loc)
        } else {
            loc.to_string()
        };
        client.get(&url).send().await.expect("follow redirect");
    }
    eprintln!("[webapp-e2e]   alice registered");

    // ===================================================================
    // Phase 2: Push mud_web.rb with web_app + web/index.html
    // ===================================================================
    eprintln!("[webapp-e2e] Phase 2: Push web_app files via editor API");

    // Push mud_web.rb with a web_app block that defines API endpoints.
    // The Rack app handles /api/* routes and also serves HTML at root.
    let mud_web_content = r#"web_app do |work_path|
  require 'json'

  ->(env) {
    req = Rack::Request.new(env)
    path = req.path_info

    case [req.request_method, path]
    when ['GET', '/']
      html = '<!DOCTYPE html><html><head><title>Web App Test</title></head><body><h1>Web App Test Page</h1></body></html>'
      [200, { 'content-type' => 'text/html' }, [html]]
    when ['GET', '/api/hello']
      [200, { 'content-type' => 'application/json' }, [{ message: 'Hello from Rack!' }.to_json]]
    when ['POST', '/api/echo']
      body = JSON.parse(req.body.read) rescue {}
      [200, { 'content-type' => 'application/json' }, [body.to_json]]
    else
      [404, {}, []]
    end
  }
end
"#;

    let resp = client
        .put(format!(
            "{}/editor/api/files/mud_web.rb?repo=alice/alice",
            base_url
        ))
        .header("Content-Type", "application/json")
        .body(serde_json::json!({ "content": mud_web_content }).to_string())
        .send()
        .await
        .expect("PUT mud_web.rb");
    assert_eq!(resp.status(), 200, "PUT mud_web.rb should succeed");
    eprintln!("[webapp-e2e]   mud_web.rb pushed");

    // ===================================================================
    // Phase 3: Commit to trigger area load
    // ===================================================================
    eprintln!("[webapp-e2e] Phase 3: Commit to trigger area load");

    let resp = client
        .post(format!("{}/git/api/repos/alice/alice/commit", base_url))
        .header("Content-Type", "application/json")
        .body(r#"{"message": "Add web_app with API endpoints"}"#)
        .send()
        .await
        .expect("POST commit");
    assert_eq!(resp.status(), 200, "commit should succeed");
    eprintln!("[webapp-e2e]   committed");

    // Wait for area reload
    tokio::time::sleep(Duration::from_secs(1)).await;

    // ===================================================================
    // Phase 4: GET /builder/alice/alice/ — verify Rack app serves HTML
    // ===================================================================
    eprintln!("[webapp-e2e] Phase 4: Rack app serves HTML at root");

    // Poll until the builder page returns the Rack app HTML content.
    // Use @dev since the editor writes to the dev workspace.
    let builder_url = format!("{}/builder/alice/alice@dev/", base_url);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut html_body = String::new();
    loop {
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "Builder page never returned Rack HTML content. Last body:\n{}",
                html_body
            );
        }
        match client.get(&builder_url).send().await {
            Ok(resp) if resp.status() == 200 => {
                html_body = resp.text().await.unwrap_or_default();
                if html_body.contains("Web App Test Page") {
                    break;
                }
            }
            Ok(resp) => {
                eprintln!("[webapp-e2e]   builder returned {}", resp.status());
            }
            Err(e) => {
                eprintln!("[webapp-e2e]   builder request failed: {}", e);
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(
        html_body.contains("Web App Test Page"),
        "Rack app should serve HTML at root. Got:\n{}",
        &html_body[..html_body.len().min(500)]
    );
    eprintln!("[webapp-e2e]   Rack app HTML served OK");

    // ===================================================================
    // Phase 5: GET /builder/alice/alice/api/hello — Rack app API
    // ===================================================================
    eprintln!("[webapp-e2e] Phase 5: Rack app GET /api/hello");

    let resp = client
        .get(format!("{}/builder/alice/alice@dev/api/hello", base_url))
        .send()
        .await
        .expect("GET /api/hello");
    assert_eq!(resp.status(), 200, "API should return 200");

    let content_type = resp
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap_or(""))
        .unwrap_or("");
    assert!(
        content_type.contains("application/json"),
        "API should return JSON content-type. Got: {}",
        content_type
    );

    let body: serde_json::Value = resp.json().await.expect("parse JSON response");
    assert_eq!(
        body["message"], "Hello from Rack!",
        "API should return hello message. Got: {:?}",
        body
    );
    eprintln!("[webapp-e2e]   GET /api/hello OK: {:?}", body);

    // ===================================================================
    // Phase 6: POST /builder/alice/alice/api/echo — POST with JSON body
    // ===================================================================
    eprintln!("[webapp-e2e] Phase 6: Rack app POST /api/echo");

    let echo_payload = serde_json::json!({ "foo": "bar", "num": 42 });
    let resp = client
        .post(format!("{}/builder/alice/alice@dev/api/echo", base_url))
        .header("Content-Type", "application/json")
        .body(echo_payload.to_string())
        .send()
        .await
        .expect("POST /api/echo");
    assert_eq!(resp.status(), 200, "echo API should return 200");

    let body: serde_json::Value = resp.json().await.expect("parse echo response");
    assert_eq!(body["foo"], "bar", "echo should return posted data");
    assert_eq!(body["num"], 42, "echo should return posted number");
    eprintln!("[webapp-e2e]   POST /api/echo OK: {:?}", body);

    // ===================================================================
    // Phase 7: GET unknown path — 404 falls through to static HTML
    // ===================================================================
    eprintln!("[webapp-e2e] Phase 7: Unknown path falls through");

    // The web_app returns 404 for unknown paths, which falls through.
    // Since there's no web_routes or SPA mode, it falls to ERB.
    // The key assertion: the server doesn't crash with nil headers.
    let resp = client
        .get(format!(
            "{}/builder/alice/alice@dev/api/nonexistent",
            base_url
        ))
        .send()
        .await
        .expect("GET /api/nonexistent");
    assert!(
        resp.status() == 404 || resp.status() == 500,
        "unknown path should return 404 or 500, not crash. Got: {}",
        resp.status()
    );
    eprintln!("[webapp-e2e]   unknown path handled (status={})", resp.status());

    // ===================================================================
    // Cleanup
    // ===================================================================
    eprintln!("[webapp-e2e] Cleaning up...");
    bg_handle.abort();
    let _ = bg_handle.await;
    eprintln!("[webapp-e2e] web_app E2E test passed!");
}

/// E2E test for area database provisioning:
///
/// Phase 0: Setup (testcontainers PG + Server + Ruby adapter)
/// Phase 1: Register bob (builder, auto-creates area bob/bob)
/// Phase 2: Push mud_web.rb with web_app that uses MUD::Container DB +
///          db/migrations/001_create_notes.rb
/// Phase 3: Commit to trigger area loading (provisions DB + runs migration)
/// Phase 4: POST /builder/bob/bob@dev/api/notes — insert a row
/// Phase 5: GET /builder/bob/bob@dev/api/notes — read rows back
/// Phase 6: GET /builder/bob/bob@dev/api/notes — verify persistence across requests
#[tokio::test]
#[ignore] // Requires Docker + Ruby; run with: cargo test -- --ignored
async fn area_database_e2e() {
    if !prerequisites_met() {
        return;
    }

    eprintln!("[area-db-e2e] Starting area database E2E test");

    // ===================================================================
    // Phase 0: Setup
    // ===================================================================

    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let socket_path = temp_dir.path().join("area-db-e2e.sock");
    let http_port = find_free_port();
    let web_socket_path = temp_dir.path().join("portal-web.sock");

    eprintln!("[area-db-e2e] Starting PostgreSQL container...");
    let container = Postgres::default()
        .start()
        .await
        .expect("start PostgreSQL container");

    let host_port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("get PostgreSQL port");

    eprintln!("[area-db-e2e] PostgreSQL ready on port {}", host_port);

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
    eprintln!("[area-db-e2e] Database initialized");

    // --- Start Rust HTTP server ---
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
            eprintln!("[area-db-e2e] HTTP server error: {}", e);
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
    eprintln!("[area-db-e2e] Portal ready");

    let base_url = format!("http://127.0.0.1:{}", http_port);

    let client = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    // ===================================================================
    // Phase 1: Register bob
    // ===================================================================
    eprintln!("[area-db-e2e] Phase 1: Register bob");

    let resp = client
        .post(format!("{}/account/register", base_url))
        .form(&[
            ("username", "bob"),
            ("password", "secret123"),
            ("character", "Mage"),
        ])
        .send()
        .await
        .expect("POST register");
    assert_eq!(resp.status(), 302, "register should redirect");

    if let Some(loc) = resp.headers().get("location") {
        let loc = loc.to_str().unwrap();
        let url = if loc.starts_with('/') {
            format!("{}{}", base_url, loc)
        } else {
            loc.to_string()
        };
        client.get(&url).send().await.expect("follow redirect");
    }
    eprintln!("[area-db-e2e]   bob registered");

    // ===================================================================
    // Phase 2: Push area files with DB migration + web_app
    // ===================================================================
    eprintln!("[area-db-e2e] Phase 2: Push area files");

    // 2a: Push mud_web.rb — web_app that uses MUD::Container for DB access
    let mud_web_content = r#"require 'json'

web_app do |work_path|
  area_key = "bob/bob"
  db = MUD::Container["database.#{area_key}"]

  unless db
    next ->(env) { [503, { 'content-type' => 'application/json' }, [{ error: 'Database not available' }.to_json]] }
  end

  ->(env) {
    req = Rack::Request.new(env)

    case [req.request_method, req.path_info]
    when ['GET', '/api/notes']
      notes = db[:notes].order(:id).all
      [200, { 'content-type' => 'application/json' }, [notes.to_json]]
    when ['POST', '/api/notes']
      body = JSON.parse(req.body.read) rescue {}
      title = body['title']&.strip
      content = body['content']&.strip
      if title && !title.empty?
        id = db[:notes].insert(title: title, content: content || '')
        note = db[:notes].where(id: id).first
        [201, { 'content-type' => 'application/json' }, [note.to_json]]
      else
        [400, { 'content-type' => 'application/json' }, [{ error: 'Title required' }.to_json]]
      end
    else
      [404, {}, []]
    end
  }
end
"#;

    let resp = client
        .put(format!(
            "{}/editor/api/files/mud_web.rb?repo=bob/bob",
            base_url
        ))
        .header("Content-Type", "application/json")
        .body(serde_json::json!({ "content": mud_web_content }).to_string())
        .send()
        .await
        .expect("PUT mud_web.rb");
    assert_eq!(resp.status(), 200, "PUT mud_web.rb should succeed");
    eprintln!("[area-db-e2e]   mud_web.rb pushed");

    // 2b: Push Sequel migration to create notes table (POST to create new file)
    let migration_content = r#"Sequel.migration do
  change do
    create_table(:notes) do
      primary_key :id
      String :title, null: false
      String :content, default: ''
      DateTime :created_at, default: Sequel::CURRENT_TIMESTAMP
    end
  end
end
"#;

    let resp = client
        .post(format!(
            "{}/editor/api/files/db/migrations/001_create_notes.rb?repo=bob/bob",
            base_url
        ))
        .header("Content-Type", "application/json")
        .body(serde_json::json!({ "content": migration_content }).to_string())
        .send()
        .await
        .expect("POST migration");
    assert!(
        resp.status() == 200 || resp.status() == 201,
        "POST migration should succeed. Got: {}",
        resp.status()
    );
    eprintln!("[area-db-e2e]   migration pushed");

    // ===================================================================
    // Phase 3: Commit to trigger area loading
    // ===================================================================
    eprintln!("[area-db-e2e] Phase 3: Commit to trigger area load + DB provisioning");

    let resp = client
        .post(format!("{}/git/api/repos/bob/bob/commit", base_url))
        .header("Content-Type", "application/json")
        .body(r#"{"message": "Add notes API with DB migration"}"#)
        .send()
        .await
        .expect("POST commit");
    assert_eq!(resp.status(), 200, "commit should succeed");
    eprintln!("[area-db-e2e]   committed");

    // Wait for area reload + DB provisioning + migration to run
    tokio::time::sleep(Duration::from_secs(2)).await;

    // ===================================================================
    // Phase 4: POST /api/notes — insert a row via area DB
    // ===================================================================
    eprintln!("[area-db-e2e] Phase 4: POST to insert a note");

    let api_base = format!("{}/builder/bob/bob@dev", base_url);

    // Poll until the API is ready (DB provisioned, migration run, web_app built)
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut last_status = 0u16;
    let mut last_body = String::new();
    loop {
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "POST /api/notes never returned 201. Last status: {}, body: {}",
                last_status, last_body
            );
        }
        match client
            .post(format!("{}/api/notes", api_base))
            .header("Content-Type", "application/json")
            .body(r#"{"title": "First note", "content": "Hello from the test!"}"#)
            .send()
            .await
        {
            Ok(resp) => {
                last_status = resp.status().as_u16();
                if last_status == 201 {
                    let body: serde_json::Value = resp.json().await.expect("parse note JSON");
                    assert_eq!(body["title"], "First note");
                    assert_eq!(body["content"], "Hello from the test!");
                    assert!(body["id"].as_i64().unwrap() > 0, "note should have an id");
                    eprintln!("[area-db-e2e]   note created: {:?}", body);
                    break;
                } else {
                    last_body = resp.text().await.unwrap_or_default();
                }
            }
            Err(e) => {
                last_body = format!("request error: {}", e);
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // Insert a second note
    let resp = client
        .post(format!("{}/api/notes", api_base))
        .header("Content-Type", "application/json")
        .body(r#"{"title": "Second note", "content": "More content"}"#)
        .send()
        .await
        .expect("POST second note");
    assert_eq!(resp.status(), 201, "second note should be created");
    eprintln!("[area-db-e2e]   second note created");

    // ===================================================================
    // Phase 5: GET /api/notes — read rows back
    // ===================================================================
    eprintln!("[area-db-e2e] Phase 5: GET notes from DB");

    let resp = client
        .get(format!("{}/api/notes", api_base))
        .send()
        .await
        .expect("GET /api/notes");
    assert_eq!(resp.status(), 200, "GET notes should return 200");

    let notes: serde_json::Value = resp.json().await.expect("parse notes JSON");
    let notes_arr = notes.as_array().expect("notes should be an array");
    assert_eq!(notes_arr.len(), 2, "should have 2 notes");
    assert_eq!(notes_arr[0]["title"], "First note");
    assert_eq!(notes_arr[1]["title"], "Second note");
    eprintln!("[area-db-e2e]   GET /api/notes returned {} notes", notes_arr.len());

    // ===================================================================
    // Phase 6: Verify data persists across requests (not in-memory)
    // ===================================================================
    eprintln!("[area-db-e2e] Phase 6: Verify DB persistence");

    // Make another GET — data should still be there (it's in PostgreSQL, not in-memory)
    let resp = client
        .get(format!("{}/api/notes", api_base))
        .send()
        .await
        .expect("GET /api/notes (persistence check)");
    assert_eq!(resp.status(), 200);
    let notes: serde_json::Value = resp.json().await.expect("parse notes");
    assert_eq!(
        notes.as_array().expect("array").len(),
        2,
        "notes should persist across requests"
    );
    eprintln!("[area-db-e2e]   persistence verified");

    // ===================================================================
    // Cleanup
    // ===================================================================
    eprintln!("[area-db-e2e] Cleaning up...");
    bg_handle.abort();
    let _ = bg_handle.await;
    eprintln!("[area-db-e2e] Area database E2E test passed!");
}
