use std::time::Duration;

use mud_core::types::AreaId;
use mud_driver::config::Config;
use mud_driver::runtime::adapter_manager::AdapterManager;
use mud_mop::codec::{read_driver_message, write_adapter_message};
use mud_mop::message::{AdapterMessage, DriverMessage, Value};
use tokio::net::UnixStream;
use tokio::time::timeout;

/// Default timeout for test operations.
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Create a Config with adapters disabled (we connect manually in tests).
fn test_config() -> Config {
    Config::default()
}

/// Helper to start an AdapterManager and return it with its listener.
/// Uses a tempdir-based socket path.
async fn start_manager() -> (AdapterManager, tokio::net::UnixListener, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let socket_path = dir.path().join("test.sock");

    let mut manager = AdapterManager::new(socket_path);
    let config = test_config();
    let listener = manager.start(&config).await.expect("start manager");

    (manager, listener, dir)
}

/// Helper: connect a fake adapter to the manager's socket, send a Handshake,
/// and return the split stream halves.
async fn connect_fake_adapter(
    socket_path: &std::path::Path,
    adapter_name: &str,
    language: &str,
    version: &str,
) -> (
    tokio::io::ReadHalf<UnixStream>,
    tokio::io::WriteHalf<UnixStream>,
) {
    let stream = UnixStream::connect(socket_path)
        .await
        .expect("connect to manager socket");
    let (read_half, mut write_half) = tokio::io::split(stream);

    // Send handshake
    let handshake = AdapterMessage::Handshake {
        adapter_name: adapter_name.into(),
        language: language.into(),
        version: version.into(),
        languages: vec![],
    };
    write_adapter_message(&mut write_half, &handshake)
        .await
        .expect("send handshake");

    (read_half, write_half)
}

// =========================================================================
// Test 1: Accept connection and handshake
// =========================================================================

#[tokio::test]
async fn accept_connection_and_handshake() {
    let (mut manager, listener, dir) = start_manager().await;
    let socket_path = dir.path().join("test.sock");

    // Spawn the fake adapter connection in a separate task
    let connect_handle = tokio::spawn(async move {
        connect_fake_adapter(&socket_path, "test-adapter", "ruby", "1.0.0").await
    });

    // Manager accepts the connection
    let result = timeout(TEST_TIMEOUT, manager.accept_connection(&listener)).await;
    let (language, additional) = result.expect("accept timed out").expect("accept failed");

    assert_eq!(language, "ruby");
    assert!(additional.is_empty());
    assert!(manager.has_adapter("ruby"));
    assert!(!manager.has_adapter("python"));

    // Clean up the fake adapter
    let (_read, _write) = connect_handle.await.expect("fake adapter task panicked");

    manager.shutdown();
}

// =========================================================================
// Test 2: Route message to adapter
// =========================================================================

#[tokio::test]
async fn route_driver_message_to_adapter() {
    let (mut manager, listener, dir) = start_manager().await;
    let socket_path = dir.path().join("test.sock");

    // Connect fake adapter in background
    let connect_handle = tokio::spawn(async move {
        connect_fake_adapter(&socket_path, "test-adapter", "ruby", "1.0.0").await
    });

    // Accept the connection
    let (language, _additional) = timeout(TEST_TIMEOUT, manager.accept_connection(&listener))
        .await
        .expect("accept timed out")
        .expect("accept failed");
    assert_eq!(language, "ruby");

    let (mut adapter_read, _adapter_write) =
        connect_handle.await.expect("fake adapter task panicked");

    // Send a LoadArea message from the driver to the adapter
    let load_msg = DriverMessage::LoadArea {
        area_id: AreaId::new("system", "lobby"),
        path: "/world/system/lobby".into(),
        db_url: None,
    };

    manager
        .send_to("ruby", load_msg.clone())
        .await
        .expect("send_to failed");

    // Fake adapter reads the message
    let received = timeout(TEST_TIMEOUT, read_driver_message(&mut adapter_read))
        .await
        .expect("read timed out")
        .expect("read failed");

    assert_eq!(load_msg, received);

    manager.shutdown();
}

#[tokio::test]
async fn route_multiple_messages_to_adapter() {
    let (mut manager, listener, dir) = start_manager().await;
    let socket_path = dir.path().join("test.sock");

    let connect_handle = tokio::spawn(async move {
        connect_fake_adapter(&socket_path, "test-adapter", "ruby", "1.0.0").await
    });

    timeout(TEST_TIMEOUT, manager.accept_connection(&listener))
        .await
        .expect("accept timed out")
        .expect("accept failed");

    let (mut adapter_read, _adapter_write) =
        connect_handle.await.expect("fake adapter task panicked");

    // Send multiple messages
    let messages = vec![
        DriverMessage::LoadArea {
            area_id: AreaId::new("system", "lobby"),
            path: "/world/system/lobby".into(),
            db_url: None,
        },
        DriverMessage::SessionStart {
            session_id: 1,
            username: "alice".into(),
        },
        DriverMessage::SessionInput {
            session_id: 1,
            line: "look".into(),
        },
    ];

    for msg in &messages {
        manager
            .send_to("ruby", msg.clone())
            .await
            .expect("send_to failed");
    }

    // Read them all back from the adapter side
    for expected in &messages {
        let received = timeout(TEST_TIMEOUT, read_driver_message(&mut adapter_read))
            .await
            .expect("read timed out")
            .expect("read failed");
        assert_eq!(expected, &received);
    }

    manager.shutdown();
}

// =========================================================================
// Test 3: Receive message from adapter
// =========================================================================

#[tokio::test]
async fn receive_message_from_adapter() {
    let (mut manager, listener, dir) = start_manager().await;
    let socket_path = dir.path().join("test.sock");

    let connect_handle = tokio::spawn(async move {
        connect_fake_adapter(&socket_path, "test-adapter", "ruby", "1.0.0").await
    });

    timeout(TEST_TIMEOUT, manager.accept_connection(&listener))
        .await
        .expect("accept timed out")
        .expect("accept failed");

    let (_adapter_read, mut adapter_write) =
        connect_handle.await.expect("fake adapter task panicked");

    // Fake adapter sends an AreaLoaded message
    let area_loaded = AdapterMessage::AreaLoaded {
        area_id: AreaId::new("system", "lobby"),
    };
    write_adapter_message(&mut adapter_write, &area_loaded)
        .await
        .expect("write failed");

    // Driver receives it
    let received = timeout(TEST_TIMEOUT, manager.recv())
        .await
        .expect("recv timed out")
        .expect("recv returned None");

    assert_eq!(area_loaded, received.message);

    manager.shutdown();
}

#[tokio::test]
async fn receive_multiple_messages_from_adapter() {
    let (mut manager, listener, dir) = start_manager().await;
    let socket_path = dir.path().join("test.sock");

    let connect_handle = tokio::spawn(async move {
        connect_fake_adapter(&socket_path, "test-adapter", "ruby", "1.0.0").await
    });

    timeout(TEST_TIMEOUT, manager.accept_connection(&listener))
        .await
        .expect("accept timed out")
        .expect("accept failed");

    let (_adapter_read, mut adapter_write) =
        connect_handle.await.expect("fake adapter task panicked");

    let messages = vec![
        AdapterMessage::AreaLoaded {
            area_id: AreaId::new("system", "lobby"),
        },
        AdapterMessage::SessionOutput {
            session_id: 1,
            text: "Welcome!\r\n".into(),
        },
        AdapterMessage::Log {
            level: "info".into(),
            message: "area loaded successfully".into(),
            area: Some("system/lobby".into()),
        },
        AdapterMessage::Pong { seq: 42 },
    ];

    // Adapter sends all messages
    for msg in &messages {
        write_adapter_message(&mut adapter_write, msg)
            .await
            .expect("write failed");
    }

    // Driver receives them in order
    for expected in &messages {
        let received = timeout(TEST_TIMEOUT, manager.recv())
            .await
            .expect("recv timed out")
            .expect("recv returned None");
        assert_eq!(expected, &received.message);
    }

    manager.shutdown();
}

// =========================================================================
// Test 4: Session flow
// =========================================================================

#[tokio::test]
async fn full_session_flow() {
    let (mut manager, listener, dir) = start_manager().await;
    let socket_path = dir.path().join("test.sock");

    let connect_handle = tokio::spawn(async move {
        connect_fake_adapter(&socket_path, "test-adapter", "ruby", "1.0.0").await
    });

    timeout(TEST_TIMEOUT, manager.accept_connection(&listener))
        .await
        .expect("accept timed out")
        .expect("accept failed");

    let (mut adapter_read, mut adapter_write) =
        connect_handle.await.expect("fake adapter task panicked");

    // --- Step 1: Driver sends SessionStart ---
    let session_start = DriverMessage::SessionStart {
        session_id: 42,
        username: "player1".into(),
    };
    manager
        .send_to("ruby", session_start.clone())
        .await
        .expect("send SessionStart");

    let received = timeout(TEST_TIMEOUT, read_driver_message(&mut adapter_read))
        .await
        .expect("read timed out")
        .expect("read failed");
    assert_eq!(session_start, received);

    // --- Step 2: Adapter sends SessionOutput (welcome message) ---
    let welcome = AdapterMessage::SessionOutput {
        session_id: 42,
        text: "Welcome to the MUD, player1!\r\n".into(),
    };
    write_adapter_message(&mut adapter_write, &welcome)
        .await
        .expect("write SessionOutput");

    let received = timeout(TEST_TIMEOUT, manager.recv())
        .await
        .expect("recv timed out")
        .expect("recv returned None");
    assert_eq!(welcome, received.message);

    // --- Step 3: Driver sends SessionInput ---
    let input = DriverMessage::SessionInput {
        session_id: 42,
        line: "look".into(),
    };
    manager
        .send_to("ruby", input.clone())
        .await
        .expect("send SessionInput");

    let received = timeout(TEST_TIMEOUT, read_driver_message(&mut adapter_read))
        .await
        .expect("read timed out")
        .expect("read failed");
    assert_eq!(input, received);

    // --- Step 4: Adapter sends room description ---
    let room_desc = AdapterMessage::SessionOutput {
        session_id: 42,
        text: "You are in a dark room. Exits: north, east.\r\n".into(),
    };
    write_adapter_message(&mut adapter_write, &room_desc)
        .await
        .expect("write room description");

    let received = timeout(TEST_TIMEOUT, manager.recv())
        .await
        .expect("recv timed out")
        .expect("recv returned None");
    assert_eq!(room_desc, received.message);

    // --- Step 5: Driver sends SessionEnd ---
    let session_end = DriverMessage::SessionEnd { session_id: 42 };
    manager
        .send_to("ruby", session_end.clone())
        .await
        .expect("send SessionEnd");

    let received = timeout(TEST_TIMEOUT, read_driver_message(&mut adapter_read))
        .await
        .expect("read timed out")
        .expect("read failed");
    assert_eq!(session_end, received);

    manager.shutdown();
}

// =========================================================================
// Test 5: Error cases
// =========================================================================

#[tokio::test]
async fn send_to_nonexistent_adapter_returns_error() {
    let (mut manager, listener, dir) = start_manager().await;
    let socket_path = dir.path().join("test.sock");

    // Connect a ruby adapter
    let connect_handle = tokio::spawn(async move {
        connect_fake_adapter(&socket_path, "test-adapter", "ruby", "1.0.0").await
    });

    timeout(TEST_TIMEOUT, manager.accept_connection(&listener))
        .await
        .expect("accept timed out")
        .expect("accept failed");

    let _ = connect_handle.await;

    // Try to send to a nonexistent "python" adapter
    let result = manager
        .send_to("python", DriverMessage::Ping { seq: 1 })
        .await;

    assert!(result.is_err());
    assert!(
        result.unwrap_err().to_string().contains("python"),
        "error should mention the missing language"
    );

    manager.shutdown();
}

// =========================================================================
// Test 6: Bidirectional message exchange (Call/CallResult)
// =========================================================================

#[tokio::test]
async fn call_and_call_result_flow() {
    let (mut manager, listener, dir) = start_manager().await;
    let socket_path = dir.path().join("test.sock");

    let connect_handle = tokio::spawn(async move {
        connect_fake_adapter(&socket_path, "test-adapter", "ruby", "1.0.0").await
    });

    timeout(TEST_TIMEOUT, manager.accept_connection(&listener))
        .await
        .expect("accept timed out")
        .expect("accept failed");

    let (mut adapter_read, mut adapter_write) =
        connect_handle.await.expect("fake adapter task panicked");

    // Driver sends a Call
    let call_msg = DriverMessage::Call {
        request_id: 7,
        object_id: 100,
        method: "get_description".into(),
        args: vec![Value::String("en".into())],
    };
    manager
        .send_to("ruby", call_msg.clone())
        .await
        .expect("send Call");

    // Adapter receives the Call
    let received = timeout(TEST_TIMEOUT, read_driver_message(&mut adapter_read))
        .await
        .expect("read timed out")
        .expect("read failed");
    assert_eq!(call_msg, received);

    // Adapter sends back a CallResult
    let result_msg = AdapterMessage::CallResult {
        request_id: 7,
        result: Value::String("A dark, musty tavern.".into()),
        cache: None,
    };
    write_adapter_message(&mut adapter_write, &result_msg)
        .await
        .expect("write CallResult");

    // Driver receives the CallResult
    let received = timeout(TEST_TIMEOUT, manager.recv())
        .await
        .expect("recv timed out")
        .expect("recv returned None");
    assert_eq!(result_msg, received.message);

    manager.shutdown();
}

// =========================================================================
// Test 7: Ping/Pong keepalive through adapter manager
// =========================================================================

#[tokio::test]
async fn ping_pong_through_adapter_manager() {
    let (mut manager, listener, dir) = start_manager().await;
    let socket_path = dir.path().join("test.sock");

    let connect_handle = tokio::spawn(async move {
        connect_fake_adapter(&socket_path, "test-adapter", "ruby", "1.0.0").await
    });

    timeout(TEST_TIMEOUT, manager.accept_connection(&listener))
        .await
        .expect("accept timed out")
        .expect("accept failed");

    let (mut adapter_read, mut adapter_write) =
        connect_handle.await.expect("fake adapter task panicked");

    // Driver sends Ping
    let ping = DriverMessage::Ping { seq: 42 };
    manager
        .send_to("ruby", ping.clone())
        .await
        .expect("send Ping");

    // Adapter receives Ping
    let received = timeout(TEST_TIMEOUT, read_driver_message(&mut adapter_read))
        .await
        .expect("read timed out")
        .expect("read failed");
    assert_eq!(ping, received);

    // Adapter sends Pong
    let pong = AdapterMessage::Pong { seq: 42 };
    write_adapter_message(&mut adapter_write, &pong)
        .await
        .expect("write Pong");

    // Driver receives Pong
    let received = timeout(TEST_TIMEOUT, manager.recv())
        .await
        .expect("recv timed out")
        .expect("recv returned None");
    assert_eq!(pong, received.message);

    manager.shutdown();
}

// =========================================================================
// Test 8: Area loading workflow through adapter manager
// =========================================================================

#[tokio::test]
async fn area_load_workflow() {
    let (mut manager, listener, dir) = start_manager().await;
    let socket_path = dir.path().join("test.sock");

    let connect_handle = tokio::spawn(async move {
        connect_fake_adapter(&socket_path, "test-adapter", "ruby", "1.0.0").await
    });

    timeout(TEST_TIMEOUT, manager.accept_connection(&listener))
        .await
        .expect("accept timed out")
        .expect("accept failed");

    let (mut adapter_read, mut adapter_write) =
        connect_handle.await.expect("fake adapter task panicked");

    // Load area
    let load = DriverMessage::LoadArea {
        area_id: AreaId::new("game", "tavern"),
        path: "/world/game/tavern".into(),
        db_url: None,
    };
    manager.send_to("ruby", load.clone()).await.unwrap();

    let received = timeout(TEST_TIMEOUT, read_driver_message(&mut adapter_read))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(load, received);

    // Adapter confirms area loaded
    let loaded = AdapterMessage::AreaLoaded {
        area_id: AreaId::new("game", "tavern"),
    };
    write_adapter_message(&mut adapter_write, &loaded)
        .await
        .unwrap();

    let received = timeout(TEST_TIMEOUT, manager.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded, received.message);

    // Reload area
    let reload = DriverMessage::ReloadArea {
        area_id: AreaId::new("game", "tavern"),
        path: "/world/game/tavern".into(),
        db_url: None,
    };
    manager.send_to("ruby", reload.clone()).await.unwrap();

    let received = timeout(TEST_TIMEOUT, read_driver_message(&mut adapter_read))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reload, received);

    // Adapter confirms reload
    write_adapter_message(&mut adapter_write, &loaded)
        .await
        .unwrap();

    let received = timeout(TEST_TIMEOUT, manager.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded, received.message);

    // Unload area
    let unload = DriverMessage::UnloadArea {
        area_id: AreaId::new("game", "tavern"),
    };
    manager.send_to("ruby", unload.clone()).await.unwrap();

    let received = timeout(TEST_TIMEOUT, read_driver_message(&mut adapter_read))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unload, received);

    manager.shutdown();
}
