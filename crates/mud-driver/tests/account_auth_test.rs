//! End-to-end integration tests for account creation and login.
//!
//! Tests the driver request handlers that back the portal account pages:
//! - `player_create` — register a new account with initial character
//! - `player_find` — look up player data (used to check username availability)
//! - `player_authenticate` — verify credentials (login)
//! - `session_create` / `session_destroy` — session token lifecycle
//! - `player_add_character` / `player_switch_character` — character management
//!
//! Uses testcontainers to spin up a PostgreSQL instance automatically.
//!
//! Run with: `cargo test -p mud-driver --test account_auth_test`

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use mud_driver::config::Config;
use mud_driver::persistence::player_store::PlayerStore;
use mud_driver::server::Server;
use mud_mop::codec::{read_driver_message, write_adapter_message};
use mud_mop::message::{AdapterMessage, DriverMessage, Value};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::UnixStream;
use tokio::time::timeout;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Test harness: testcontainers PostgreSQL + Server + fake adapter
// ---------------------------------------------------------------------------

struct TestHarness {
    server: Server,
    adapter_read: ReadHalf<UnixStream>,
    adapter_write: WriteHalf<UnixStream>,
    #[allow(dead_code)]
    player_store: Arc<PlayerStore>,
    _temp_dir: tempfile::TempDir,
    // Keep the container alive for the test duration.
    _container: testcontainers::ContainerAsync<Postgres>,
}

async fn build_harness() -> TestHarness {
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let socket_path = temp_dir.path().join("test.sock");

    // Start a PostgreSQL container via testcontainers.
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

    // Set up server with fake adapter and handshake.
    let mut server = Server::new_with_socket_path(Config::default(), socket_path.clone());
    server.set_player_store(Arc::clone(&player_store));

    let listener = server
        .start_adapter_manager()
        .await
        .expect("start adapter manager");

    // Spawn fake adapter: connect + send handshake concurrently with accept.
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
            languages: vec![],
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
        _temp_dir: temp_dir,
        _container: container,
    }
}

// ---------------------------------------------------------------------------
// Helper: make params Value::Map
// ---------------------------------------------------------------------------

fn params(entries: &[(&str, Value)]) -> Value {
    let mut map = HashMap::new();
    for (k, v) in entries {
        map.insert((*k).to_string(), v.clone());
    }
    Value::Map(map)
}

/// Helper: send a driver request from the fake adapter, have the server
/// process it, and return the response.
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

// ===========================================================================
// Tests
// ===========================================================================

#[tokio::test]
async fn full_account_lifecycle() {
    let mut h = build_harness().await;

    // -- Step 1: player_find for nonexistent user → null
    let resp = round_trip(
        &mut h,
        1,
        "player_find",
        params(&[("username", Value::String("alice".into()))]),
    )
    .await;
    match resp {
        DriverMessage::RequestResponse { request_id, result } => {
            assert_eq!(request_id, 1);
            assert_eq!(result, Value::Null, "nonexistent user should return null");
        }
        other => panic!("expected RequestResponse, got: {:?}", other),
    }

    // -- Step 2: player_create — register "alice" with character "Warrior"
    let resp = round_trip(
        &mut h,
        2,
        "player_create",
        params(&[
            ("username", Value::String("alice".into())),
            ("password", Value::String("secret123".into())),
            ("character", Value::String("Warrior".into())),
        ]),
    )
    .await;
    match resp {
        DriverMessage::RequestResponse { request_id, result } => {
            assert_eq!(request_id, 2);
            assert_eq!(result, Value::Bool(true));
        }
        other => panic!("expected RequestResponse, got: {:?}", other),
    }

    // -- Step 3: player_find — "alice" should exist with character data
    let resp = round_trip(
        &mut h,
        3,
        "player_find",
        params(&[("username", Value::String("alice".into()))]),
    )
    .await;
    match &resp {
        DriverMessage::RequestResponse { request_id, result } => {
            assert_eq!(*request_id, 3);
            if let Value::Map(map) = result {
                assert_eq!(map.get("id"), Some(&Value::String("alice".into())));
                assert_eq!(map.get("role"), Some(&Value::String("builder".into())));
                assert_eq!(
                    map.get("active_character"),
                    Some(&Value::String("Warrior".into()))
                );
                if let Some(Value::Array(chars)) = map.get("characters") {
                    assert_eq!(chars.len(), 1);
                    if let Some(Value::Map(c)) = chars.first() {
                        assert_eq!(c.get("name"), Some(&Value::String("Warrior".into())));
                    } else {
                        panic!("character should be a map");
                    }
                } else {
                    panic!("characters should be an array");
                }
            } else {
                panic!("expected Map, got: {:?}", result);
            }
        }
        other => panic!("expected RequestResponse, got: {:?}", other),
    }

    // -- Step 4: player_authenticate — correct password → success
    let resp = round_trip(
        &mut h,
        4,
        "player_authenticate",
        params(&[
            ("username", Value::String("alice".into())),
            ("password", Value::String("secret123".into())),
        ]),
    )
    .await;
    match &resp {
        DriverMessage::RequestResponse { request_id, result } => {
            assert_eq!(*request_id, 4);
            if let Value::Map(map) = result {
                assert_eq!(map.get("success"), Some(&Value::Bool(true)));
                if let Some(Value::Map(data)) = map.get("data") {
                    assert_eq!(data.get("role"), Some(&Value::String("builder".into())));
                    assert_eq!(
                        data.get("active_character"),
                        Some(&Value::String("Warrior".into()))
                    );
                } else {
                    panic!("expected data map in auth response");
                }
            } else {
                panic!("expected Map, got: {:?}", result);
            }
        }
        other => panic!("expected RequestResponse, got: {:?}", other),
    }

    // -- Step 5: player_authenticate — wrong password → failure
    let resp = round_trip(
        &mut h,
        5,
        "player_authenticate",
        params(&[
            ("username", Value::String("alice".into())),
            ("password", Value::String("wrongpass".into())),
        ]),
    )
    .await;
    match &resp {
        DriverMessage::RequestResponse { request_id, result } => {
            assert_eq!(*request_id, 5);
            if let Value::Map(map) = result {
                assert_eq!(map.get("success"), Some(&Value::Bool(false)));
            } else {
                panic!("expected Map, got: {:?}", result);
            }
        }
        other => panic!("expected RequestResponse, got: {:?}", other),
    }

    // -- Step 6: session_create → get session token
    let resp = round_trip(
        &mut h,
        6,
        "session_create",
        params(&[("account", Value::String("alice".into()))]),
    )
    .await;
    let session_token = match &resp {
        DriverMessage::RequestResponse { request_id, result } => {
            assert_eq!(*request_id, 6);
            match result {
                Value::String(token) => {
                    assert_eq!(token.len(), 64, "session token should be 64 hex chars");
                    token.clone()
                }
                other => panic!("expected String token, got: {:?}", other),
            }
        }
        other => panic!("expected RequestResponse, got: {:?}", other),
    };

    // -- Step 7: session_destroy → destroys the session
    let resp = round_trip(
        &mut h,
        7,
        "session_destroy",
        params(&[("token", Value::String(session_token))]),
    )
    .await;
    match resp {
        DriverMessage::RequestResponse { request_id, result } => {
            assert_eq!(request_id, 7);
            assert_eq!(result, Value::Bool(true));
        }
        other => panic!("expected RequestResponse, got: {:?}", other),
    }

    // -- Step 8: player_add_character → add "Mage"
    let resp = round_trip(
        &mut h,
        8,
        "player_add_character",
        params(&[
            ("account", Value::String("alice".into())),
            ("name", Value::String("Mage".into())),
        ]),
    )
    .await;
    match resp {
        DriverMessage::RequestResponse { request_id, result } => {
            assert_eq!(request_id, 8);
            assert_eq!(result, Value::Bool(true));
        }
        other => panic!("expected RequestResponse, got: {:?}", other),
    }

    // -- Step 9: player_switch_character → switch to "Mage"
    let resp = round_trip(
        &mut h,
        9,
        "player_switch_character",
        params(&[
            ("account", Value::String("alice".into())),
            ("character", Value::String("Mage".into())),
        ]),
    )
    .await;
    match resp {
        DriverMessage::RequestResponse { request_id, result } => {
            assert_eq!(request_id, 9);
            assert_eq!(result, Value::Bool(true));
        }
        other => panic!("expected RequestResponse, got: {:?}", other),
    }

    // -- Step 10: Verify switch — player_find should show Mage active + 2 chars
    let resp = round_trip(
        &mut h,
        10,
        "player_find",
        params(&[("username", Value::String("alice".into()))]),
    )
    .await;
    match &resp {
        DriverMessage::RequestResponse { request_id, result } => {
            assert_eq!(*request_id, 10);
            if let Value::Map(map) = result {
                assert_eq!(
                    map.get("active_character"),
                    Some(&Value::String("Mage".into())),
                    "active character should be Mage after switch"
                );
                if let Some(Value::Array(chars)) = map.get("characters") {
                    assert_eq!(chars.len(), 2, "should have 2 characters");
                } else {
                    panic!("characters should be an array");
                }
            } else {
                panic!("expected Map, got: {:?}", result);
            }
        }
        other => panic!("expected RequestResponse, got: {:?}", other),
    }

    // -- Step 11: Duplicate player_create → error
    let resp = round_trip(
        &mut h,
        11,
        "player_create",
        params(&[
            ("username", Value::String("alice".into())),
            ("password", Value::String("other".into())),
            ("character", Value::String("Rogue".into())),
        ]),
    )
    .await;
    match resp {
        DriverMessage::RequestError { request_id, error } => {
            assert_eq!(request_id, 11);
            assert!(
                error.contains("failed to create player"),
                "duplicate should error: {}",
                error
            );
        }
        other => panic!("expected RequestError for duplicate, got: {:?}", other),
    }

    // -- Step 12: Unknown action → error
    let resp = round_trip(&mut h, 12, "nonexistent_action", Value::Null).await;
    match resp {
        DriverMessage::RequestError { request_id, error } => {
            assert_eq!(request_id, 12);
            assert!(error.contains("unknown action"), "got: {}", error);
        }
        other => panic!("expected RequestError, got: {:?}", other),
    }

    eprintln!("[e2e] Full account lifecycle test passed (12 steps)!");
    h.server.shutdown();
}
