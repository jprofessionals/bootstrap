//! Comprehensive full-stack E2E test covering the entire lifecycle:
//!
//! 1. Account creation and builder promotion (MOP round-trips)
//! 2. Git repository creation and ACL management
//! 3. HTTP git authentication (Axum tower::oneshot)
//! 4. Workspace checkout, modification, and commit
//! 5. Area reload via MOP → adapter ReloadArea message
//!
//! Combines patterns from `account_auth_test.rs` (fake adapter + MOP)
//! and `http_git_test.rs` (Axum tower::oneshot for HTTP git).
//!
//! Run with: `cargo test -p mud-driver --test full_stack_e2e_test -- --nocapture`

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::Engine;
use tower::ServiceExt;

use mud_driver::config::Config;
use mud_driver::git::repo_manager::{AccessLevel, RepoManager};
use mud_driver::git::workspace::Workspace;
use mud_driver::persistence::player_store::PlayerStore;
use mud_driver::server::Server;
use mud_driver::web::git_http::git_http_routes;
use mud_driver::web::server::AppState;
use mud_mop::codec::{read_driver_message, write_adapter_message};
use mud_mop::message::{AdapterMessage, DriverMessage, Value};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::UnixStream;
use tokio::time::timeout;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

struct TestHarness {
    server: Server,
    adapter_read: ReadHalf<UnixStream>,
    adapter_write: WriteHalf<UnixStream>,
    #[allow(dead_code)]
    player_store: Arc<PlayerStore>,
    repo_manager: Arc<RepoManager>,
    workspace: Arc<Workspace>,
    app: Router,
    _temp_dir: tempfile::TempDir,
    _container: testcontainers::ContainerAsync<Postgres>,
}

async fn build_harness() -> TestHarness {
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let socket_path = temp_dir.path().join("test.sock");
    let repos_path = temp_dir.path().join("repos");
    let world_path = temp_dir.path().join("world");
    std::fs::create_dir_all(&repos_path).unwrap();
    std::fs::create_dir_all(&world_path).unwrap();

    // Start PostgreSQL container.
    let container = Postgres::default()
        .start()
        .await
        .expect("start PostgreSQL container");

    let host_port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("get PostgreSQL port");

    let db_url = format!(
        "postgres://postgres:postgres@127.0.0.1:{}/postgres",
        host_port
    );

    // Connect and create stdlib tables.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("connect to test PostgreSQL");

    for sql in &[
        "CREATE TABLE IF NOT EXISTS players (
            id VARCHAR PRIMARY KEY,
            password_hash VARCHAR NOT NULL,
            role VARCHAR NOT NULL DEFAULT 'builder',
            active_character VARCHAR,
            builder_character_id INTEGER,
            created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
        )",
        "CREATE TABLE IF NOT EXISTS characters (
            id SERIAL PRIMARY KEY,
            player_id VARCHAR NOT NULL REFERENCES players(id),
            name VARCHAR NOT NULL,
            created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
        )",
        "CREATE TABLE IF NOT EXISTS sessions (
            id SERIAL PRIMARY KEY,
            player_id VARCHAR NOT NULL REFERENCES players(id),
            token VARCHAR NOT NULL,
            created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
        )",
        "CREATE TABLE IF NOT EXISTS access_tokens (
            id SERIAL PRIMARY KEY,
            player_id VARCHAR NOT NULL REFERENCES players(id),
            name VARCHAR NOT NULL,
            token_prefix VARCHAR NOT NULL,
            token_hash VARCHAR NOT NULL,
            created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
            last_used_at TIMESTAMPTZ
        )",
    ] {
        sqlx::query(sql)
            .execute(&pool)
            .await
            .expect("create table");
    }

    let player_store = Arc::new(PlayerStore::new(pool));
    let repo_manager = Arc::new(RepoManager::new(repos_path));
    let workspace = Arc::new(Workspace::new(world_path, Arc::clone(&repo_manager)));

    // Build Axum router for HTTP git tests.
    let templates = Arc::new(tera::Tera::default());
    let state = AppState {
        player_store: Arc::clone(&player_store),
        repo_manager: Arc::clone(&repo_manager),
        workspace: Arc::clone(&workspace),
        templates,
        ai_key_store: None,
        skills_service: None,
        http_client: reqwest::Client::new(),
        portal_socket: "/tmp/mud-portal.sock".into(),
        anthropic_base_url: None,
    };
    let app = Router::new()
        .nest("/git", git_http_routes())
        .with_state(state);

    // Set up Server with fake adapter.
    let mut server = Server::new_with_socket_path(Config::default(), socket_path.clone());
    server.set_player_store(Arc::clone(&player_store));
    server.set_repo_manager(Arc::clone(&repo_manager));
    server.set_workspace(Arc::clone(&workspace));

    let listener = server
        .start_adapter_manager()
        .await
        .expect("start adapter manager");

    // Spawn fake adapter: connect + handshake.
    let socket_path_clone = socket_path.clone();
    let connect_handle = tokio::spawn(async move {
        let stream = UnixStream::connect(&socket_path_clone)
            .await
            .expect("connect to socket");
        let (read_half, mut write_half) = tokio::io::split(stream);

        let handshake = AdapterMessage::Handshake {
            adapter_name: "test-adapter".into(),
            language: "ruby".into(),
            version: "1.0.0".into(),
        };
        write_adapter_message(&mut write_half, &handshake)
            .await
            .expect("send handshake");

        (read_half, write_half)
    });

    let language = timeout(TEST_TIMEOUT, server.accept_adapter(&listener))
        .await
        .expect("accept timed out")
        .expect("accept failed");
    assert_eq!(language, "ruby");

    let (adapter_read, adapter_write) = connect_handle.await.expect("connect task");

    TestHarness {
        server,
        adapter_read,
        adapter_write,
        player_store,
        repo_manager,
        workspace,
        app,
        _temp_dir: temp_dir,
        _container: container,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn params(entries: &[(&str, Value)]) -> Value {
    let mut map = HashMap::new();
    for (k, v) in entries {
        map.insert((*k).to_string(), v.clone());
    }
    Value::Map(map)
}

async fn round_trip(
    h: &mut TestHarness,
    request_id: u64,
    action: &str,
    request_params: Value,
) -> DriverMessage {
    write_adapter_message(
        &mut h.adapter_write,
        &AdapterMessage::DriverRequest {
            request_id,
            action: action.into(),
            params: request_params,
        },
    )
    .await
    .expect("send request");

    let msg = timeout(TEST_TIMEOUT, h.server.recv_adapter_message())
        .await
        .expect("recv timed out")
        .expect("recv None");
    h.server.handle_adapter_message(msg).await;

    timeout(TEST_TIMEOUT, read_driver_message(&mut h.adapter_read))
        .await
        .expect("read response timed out")
        .expect("read response failed")
}

fn basic_auth(username: &str, password: &str) -> String {
    let encoded =
        base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", username, password));
    format!("Basic {}", encoded)
}

fn assert_request_success(resp: &DriverMessage, expected_id: u64) -> &Value {
    match resp {
        DriverMessage::RequestResponse { request_id, result } => {
            assert_eq!(*request_id, expected_id);
            result
        }
        other => panic!("expected RequestResponse, got: {:?}", other),
    }
}

// ===========================================================================
// The Test
// ===========================================================================

#[tokio::test]
async fn full_stack_e2e() {
    let mut h = build_harness().await;
    let mut req_id: u64 = 0;

    let mut next_id = || {
        req_id += 1;
        req_id
    };

    // -----------------------------------------------------------------------
    // Phase 1: Account creation and builder promotion
    // -----------------------------------------------------------------------
    eprintln!("[e2e] Phase 1: Account creation and builder promotion");

    // Step 1: Create account "alice" with character "Warrior"
    let id = next_id();
    let resp = round_trip(
        &mut h,
        id,
        "player_create",
        params(&[
            ("username", Value::String("alice".into())),
            ("password", Value::String("secret123".into())),
            ("character", Value::String("Warrior".into())),
        ]),
    )
    .await;
    let result = assert_request_success(&resp, id);
    assert_eq!(*result, Value::Bool(true));
    eprintln!("[e2e]   Step 1: Created account 'alice' with character 'Warrior'");

    // Step 2: Promote alice to builder
    let id = next_id();
    let resp = round_trip(
        &mut h,
        id,
        "set_role",
        params(&[
            ("username", Value::String("alice".into())),
            ("role", Value::String("builder".into())),
        ]),
    )
    .await;
    let result = assert_request_success(&resp, id);
    assert_eq!(*result, Value::Bool(true));
    eprintln!("[e2e]   Step 2: Promoted alice to builder");

    // Step 3: Verify role change via player_find
    let id = next_id();
    let resp = round_trip(
        &mut h,
        id,
        "player_find",
        params(&[("username", Value::String("alice".into()))]),
    )
    .await;
    let result = assert_request_success(&resp, id);
    if let Value::Map(map) = result {
        assert_eq!(map.get("role"), Some(&Value::String("builder".into())));
        assert_eq!(
            map.get("active_character"),
            Some(&Value::String("Warrior".into()))
        );
    } else {
        panic!("expected Map, got: {:?}", result);
    }
    eprintln!("[e2e]   Step 3: Verified alice is now a builder");

    // -----------------------------------------------------------------------
    // Phase 2: Git repository creation
    // -----------------------------------------------------------------------
    eprintln!("[e2e] Phase 2: Git repository creation");

    // Step 4: Create repo via MOP
    let id = next_id();
    let resp = round_trip(
        &mut h,
        id,
        "repo_create",
        params(&[
            ("namespace", Value::String("testns".into())),
            ("name", Value::String("village".into())),
        ]),
    )
    .await;
    let result = assert_request_success(&resp, id);
    assert_eq!(*result, Value::Bool(true));
    eprintln!("[e2e]   Step 4: Created repo testns/village");

    // Step 5: List repos via MOP
    let id = next_id();
    let resp = round_trip(
        &mut h,
        id,
        "repo_list",
        params(&[("namespace", Value::String("testns".into()))]),
    )
    .await;
    let result = assert_request_success(&resp, id);
    if let Value::Array(repos) = result {
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0], Value::String("village".into()));
    } else {
        panic!("expected Array, got: {:?}", result);
    }
    eprintln!("[e2e]   Step 5: Listed repos in testns — found 'village'");

    // Step 6: Grant alice read_write access
    h.repo_manager
        .grant_access("testns", "village", "alice", AccessLevel::ReadWrite)
        .unwrap();
    eprintln!("[e2e]   Step 6: Granted alice ReadWrite access");

    // -----------------------------------------------------------------------
    // Phase 3: HTTP git authentication
    // -----------------------------------------------------------------------
    eprintln!("[e2e] Phase 3: HTTP git authentication");

    // Step 7: HTTP git auth succeeds with valid credentials
    let request = Request::builder()
        .uri("/git/testns/village.git/info/refs?service=git-upload-pack")
        .header("Authorization", basic_auth("alice", "secret123"))
        .body(Body::empty())
        .unwrap();
    let response = h
        .app
        .clone()
        .oneshot(request)
        .await
        .expect("HTTP request failed");
    assert_eq!(response.status(), StatusCode::OK);
    eprintln!("[e2e]   Step 7: HTTP git auth succeeded (200 OK)");

    // Step 8: HTTP git auth fails without credentials
    let request = Request::builder()
        .uri("/git/testns/village.git/info/refs?service=git-upload-pack")
        .body(Body::empty())
        .unwrap();
    let response = h
        .app
        .clone()
        .oneshot(request)
        .await
        .expect("HTTP request failed");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    eprintln!("[e2e]   Step 8: HTTP git auth rejected without credentials (401)");

    // Step 9: HTTP git auth fails with wrong password
    let request = Request::builder()
        .uri("/git/testns/village.git/info/refs?service=git-upload-pack")
        .header("Authorization", basic_auth("alice", "wrongpass"))
        .body(Body::empty())
        .unwrap();
    let response = h
        .app
        .clone()
        .oneshot(request)
        .await
        .expect("HTTP request failed");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    eprintln!("[e2e]   Step 9: HTTP git auth rejected with wrong password (401)");

    // -----------------------------------------------------------------------
    // Phase 4: Workspace checkout and modification
    // -----------------------------------------------------------------------
    eprintln!("[e2e] Phase 4: Workspace checkout and modification");

    // Step 10: Checkout workspace
    let prod_path = h.workspace.checkout("testns", "village").unwrap();
    assert!(prod_path.exists());
    let dev_path = h.workspace.dev_path("testns", "village");
    assert!(dev_path.exists());
    eprintln!("[e2e]   Step 10: Checked out both production and dev working copies");

    // Step 11: Verify seed files in production
    let entrance_path = prod_path.join("rooms/entrance.rb");
    assert!(entrance_path.exists(), "seed file rooms/entrance.rb should exist");
    let entrance_content = std::fs::read_to_string(&entrance_path).unwrap();
    assert!(
        entrance_content.contains("class Entrance"),
        "entrance.rb should contain seeded room class"
    );
    assert!(
        entrance_content.contains("Welcome to village"),
        "entrance.rb should contain area name in description"
    );
    eprintln!("[e2e]   Step 11: Verified seed files in production checkout");

    // Step 12: Modify file in develop checkout
    let dev_entrance_path = dev_path.join("rooms/entrance.rb");
    let modified_content = "class Entrance < Room\n  title \"The Village Gate\"\n  description \"A heavily modified entrance.\"\n  exit :north, to: \"hall\"\nend\n";
    std::fs::write(&dev_entrance_path, modified_content).unwrap();
    eprintln!("[e2e]   Step 12: Modified rooms/entrance.rb in dev checkout");

    // Step 13: Commit and push develop
    let oid = h
        .workspace
        .commit("testns", "village", "alice", "Modify entrance room", "develop")
        .unwrap();
    assert!(!oid.is_empty());
    eprintln!("[e2e]   Step 13: Committed and pushed develop ({})", &oid[..8]);

    // Step 14: Verify production is unchanged
    let prod_content = std::fs::read_to_string(&entrance_path).unwrap();
    assert!(
        prod_content.contains("Welcome to village"),
        "production should still have original content"
    );
    assert!(
        !prod_content.contains("heavily modified"),
        "production should NOT have dev changes"
    );
    eprintln!("[e2e]   Step 14: Verified production unchanged after dev commit");

    // Step 15: Verify dev commit is in log
    let commits = h.workspace.log("testns", "village", "develop", 10).unwrap();
    assert!(commits.len() >= 2, "should have seed + new commit");
    assert_eq!(commits[0].message, "Modify entrance room");
    assert_eq!(commits[0].author, "alice");
    eprintln!("[e2e]   Step 15: Verified dev commit in log");

    // -----------------------------------------------------------------------
    // Phase 5: Area reload via MOP
    // -----------------------------------------------------------------------
    eprintln!("[e2e] Phase 5: Area reload via MOP");

    // Step 16: Send area_reload request — this generates TWO messages to the
    // adapter: ReloadArea (from the handler) and RequestResponse (the reply).
    // We can't use round_trip here because it only reads one response.
    let id = next_id();
    write_adapter_message(
        &mut h.adapter_write,
        &AdapterMessage::DriverRequest {
            request_id: id,
            action: "area_reload".into(),
            params: params(&[
                ("area_id", Value::String("testns/village".into())),
                ("path", Value::String(prod_path.to_string_lossy().into())),
            ]),
        },
    )
    .await
    .expect("send area_reload request");

    // Server processes the message.
    let msg = timeout(TEST_TIMEOUT, h.server.recv_adapter_message())
        .await
        .expect("recv timed out")
        .expect("recv None");
    h.server.handle_adapter_message(msg).await;

    // Read both messages from the adapter stream (order may vary).
    let msg1 = timeout(TEST_TIMEOUT, read_driver_message(&mut h.adapter_read))
        .await
        .expect("read msg1 timed out")
        .expect("read msg1 failed");
    let msg2 = timeout(TEST_TIMEOUT, read_driver_message(&mut h.adapter_read))
        .await
        .expect("read msg2 timed out")
        .expect("read msg2 failed");

    // One should be ReloadArea, the other RequestResponse.
    let (reload_msg, response_msg) = match (&msg1, &msg2) {
        (DriverMessage::ReloadArea { .. }, DriverMessage::RequestResponse { .. }) => (&msg1, &msg2),
        (DriverMessage::RequestResponse { .. }, DriverMessage::ReloadArea { .. }) => (&msg2, &msg1),
        _ => panic!(
            "expected one ReloadArea and one RequestResponse, got: {:?} and {:?}",
            msg1, msg2
        ),
    };

    match reload_msg {
        DriverMessage::ReloadArea { area_id, path, .. } => {
            assert_eq!(area_id.namespace, "testns");
            assert_eq!(area_id.name, "village");
            assert!(!path.is_empty());
        }
        _ => unreachable!(),
    }
    let result = assert_request_success(response_msg, id);
    assert_eq!(*result, Value::Bool(true));
    eprintln!("[e2e]   Step 16-17: area_reload succeeded and adapter received ReloadArea");

    // -----------------------------------------------------------------------
    // Phase 6: Second area (SPA-like) creation and structure verification
    // -----------------------------------------------------------------------
    eprintln!("[e2e] Phase 6: SPA-like area creation");

    // Step 18: Create second repo
    let id = next_id();
    let resp = round_trip(
        &mut h,
        id,
        "repo_create",
        params(&[
            ("namespace", Value::String("testns".into())),
            ("name", Value::String("spa_demo".into())),
        ]),
    )
    .await;
    let result = assert_request_success(&resp, id);
    assert_eq!(*result, Value::Bool(true));
    eprintln!("[e2e]   Step 18: Created repo testns/spa_demo");

    // Step 19: Checkout and write SPA files in dev
    h.workspace.checkout("testns", "spa_demo").unwrap();
    let spa_dev_path = h.workspace.dev_path("testns", "spa_demo");

    std::fs::write(
        spa_dev_path.join("mud_web.rb"),
        "web_mode :spa\n\nweb_routes do |r|\n  r.get \"status\" do\n    { status: \"ok\" }\n  end\nend\n",
    )
    .unwrap();
    std::fs::write(
        spa_dev_path.join("package.json"),
        r#"{"name":"spa_demo","version":"1.0.0","dependencies":{}}"#,
    )
    .unwrap();
    std::fs::create_dir_all(spa_dev_path.join("src")).unwrap();
    std::fs::write(
        spa_dev_path.join("src/App.jsx"),
        "export default function App() { return <h1>Hello MUD</h1>; }\n",
    )
    .unwrap();
    eprintln!("[e2e]   Step 19: Wrote SPA files in dev checkout");

    // Step 20: Commit SPA files
    let oid = h
        .workspace
        .commit("testns", "spa_demo", "alice", "Add SPA scaffold", "develop")
        .unwrap();
    assert!(!oid.is_empty());
    eprintln!("[e2e]   Step 20: Committed SPA files ({})", &oid[..8]);

    // Step 21: Verify SPA files exist in dev
    assert!(spa_dev_path.join("mud_web.rb").exists());
    assert!(spa_dev_path.join("package.json").exists());
    assert!(spa_dev_path.join("src/App.jsx").exists());
    let mud_web = std::fs::read_to_string(spa_dev_path.join("mud_web.rb")).unwrap();
    assert!(mud_web.contains("web_mode :spa"));
    eprintln!("[e2e]   Step 21: Verified SPA area structure");

    // Step 22: Verify repo_list now shows both repos
    let id = next_id();
    let resp = round_trip(
        &mut h,
        id,
        "repo_list",
        params(&[("namespace", Value::String("testns".into()))]),
    )
    .await;
    let result = assert_request_success(&resp, id);
    if let Value::Array(repos) = result {
        assert_eq!(repos.len(), 2);
        let names: Vec<&str> = repos
            .iter()
            .map(|v| match v {
                Value::String(s) => s.as_str(),
                _ => panic!("expected String in repo list"),
            })
            .collect();
        assert!(names.contains(&"village"));
        assert!(names.contains(&"spa_demo"));
    } else {
        panic!("expected Array, got: {:?}", result);
    }
    eprintln!("[e2e]   Step 22: Verified repo_list shows both repos");

    // -----------------------------------------------------------------------
    // Cleanup
    // -----------------------------------------------------------------------
    h.server.shutdown();
    eprintln!("[e2e] Full-stack E2E test passed (22 steps)!");
}
