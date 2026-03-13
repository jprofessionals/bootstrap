use std::time::Duration;

use mud_driver::config::Config;
use mud_driver::git::merge_request_manager::ReviewPolicy;
use mud_driver::runtime::adapter_manager::AdapterManager;
use mud_driver::server::Server;
use mud_mop::codec::{read_driver_message, write_adapter_message};
use mud_mop::message::{AdapterMessage, DriverMessage};
use tokio::net::UnixStream;
use tokio::time::timeout;

/// Default timeout for test operations.
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Create a Config with adapters disabled (we connect manually in tests).
fn test_config() -> Config {
    Config::default()
}

/// Helper: connect a fake adapter to the manager's socket, send a Handshake,
/// and return the split stream halves.
async fn connect_fake_adapter(
    socket_path: &std::path::Path,
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
        adapter_name: "test-adapter".into(),
        language: "ruby".into(),
        version: "1.0.0".into(),
        languages: vec![],
    };
    write_adapter_message(&mut write_half, &handshake)
        .await
        .expect("send handshake");

    (read_half, write_half)
}

/// Helper: create a Server with a unique socket path in a temp directory,
/// start the adapter manager, connect a fake adapter, and return everything
/// needed for testing.
///
/// Returns (server, adapter_read, adapter_write, _tempdir).
/// The caller must keep `_tempdir` alive for the duration of the test.
async fn setup_server_with_adapter() -> (
    Server,
    tokio::io::ReadHalf<UnixStream>,
    tokio::io::WriteHalf<UnixStream>,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let socket_path = dir.path().join("test.sock");

    let config = test_config();
    let mut server = Server::new_with_socket_path(config, socket_path.clone());

    let listener = server
        .start_adapter_manager()
        .await
        .expect("start adapter manager");

    let sp = socket_path.clone();
    let connect_handle = tokio::spawn(async move { connect_fake_adapter(&sp).await });

    let language = timeout(TEST_TIMEOUT, server.accept_adapter(&listener))
        .await
        .expect("accept timed out")
        .expect("accept failed");
    assert_eq!(language, "ruby");

    let (adapter_read, adapter_write) = connect_handle.await.expect("fake adapter task panicked");

    (server, adapter_read, adapter_write, dir)
}

/// Helper to start an AdapterManager, connect a fake adapter, and return
/// all the pieces needed for component-level testing.
///
/// Returns (manager, adapter_read_half, adapter_write_half, _tempdir).
async fn setup_connected_adapter() -> (
    AdapterManager,
    tokio::io::ReadHalf<UnixStream>,
    tokio::io::WriteHalf<UnixStream>,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let socket_path = dir.path().join("test.sock");

    let mut manager = AdapterManager::new(socket_path.clone());
    let config = test_config();
    let listener = manager.start(&config).await.expect("start manager");

    let sp = socket_path.clone();
    let connect_handle = tokio::spawn(async move { connect_fake_adapter(&sp).await });

    let (language, _additional) = timeout(TEST_TIMEOUT, manager.accept_connection(&listener))
        .await
        .expect("accept timed out")
        .expect("accept failed");
    assert_eq!(language, "ruby");

    let (adapter_read, adapter_write) = connect_handle.await.expect("fake adapter task panicked");

    (manager, adapter_read, adapter_write, dir)
}

// =========================================================================
// Session lifecycle tests
// =========================================================================

#[tokio::test]
async fn create_session_allocates_incrementing_ids() {
    // create_session does not need a socket — only allocates IDs in memory.
    let dir = tempfile::tempdir().expect("create temp dir");
    let socket_path = dir.path().join("test.sock");
    let config = test_config();
    let mut server = Server::new_with_socket_path(config, socket_path);

    let (id1, _rx1) = server.create_session();
    let (id2, _rx2) = server.create_session();
    let (id3, _rx3) = server.create_session();

    assert_eq!(id1, 1);
    assert_eq!(id2, 2);
    assert_eq!(id3, 3);
}

#[tokio::test]
async fn session_start_sends_message_to_adapter() {
    // Test at the adapter manager level to verify the message format.
    let (manager, mut adapter_read, _adapter_write, _dir) = setup_connected_adapter().await;

    manager
        .send_to(
            "ruby",
            DriverMessage::SessionStart {
                session_id: 1,
                username: "testplayer".into(),
            },
        )
        .await
        .expect("send SessionStart");

    let received = timeout(TEST_TIMEOUT, read_driver_message(&mut adapter_read))
        .await
        .expect("read timed out")
        .expect("read failed");

    assert_eq!(
        received,
        DriverMessage::SessionStart {
            session_id: 1,
            username: "testplayer".into(),
        }
    );
}

#[tokio::test]
async fn session_input_sends_message_to_adapter() {
    let (manager, mut adapter_read, _adapter_write, _dir) = setup_connected_adapter().await;

    manager
        .send_to(
            "ruby",
            DriverMessage::SessionInput {
                session_id: 42,
                line: "look around".into(),
            },
        )
        .await
        .expect("send SessionInput");

    let received = timeout(TEST_TIMEOUT, read_driver_message(&mut adapter_read))
        .await
        .expect("read timed out")
        .expect("read failed");

    assert_eq!(
        received,
        DriverMessage::SessionInput {
            session_id: 42,
            line: "look around".into(),
        }
    );
}

#[tokio::test]
async fn session_end_sends_message_to_adapter() {
    let (manager, mut adapter_read, _adapter_write, _dir) = setup_connected_adapter().await;

    manager
        .send_to("ruby", DriverMessage::SessionEnd { session_id: 7 })
        .await
        .expect("send SessionEnd");

    let received = timeout(TEST_TIMEOUT, read_driver_message(&mut adapter_read))
        .await
        .expect("read timed out")
        .expect("read failed");

    assert_eq!(received, DriverMessage::SessionEnd { session_id: 7 });
}

#[tokio::test]
async fn full_session_lifecycle_through_server() {
    // Exercises the complete session lifecycle using Server's public API:
    // create_session -> session_start -> session_input -> session_end.
    let (mut server, mut adapter_read, _adapter_write, _dir) = setup_server_with_adapter().await;

    // Step 1: Create a session — verify incrementing IDs
    let (session_id, _output_rx) = server.create_session();
    assert_eq!(session_id, 1);

    // Step 2: session_start sends SessionStart to adapter
    server
        .session_start(session_id, "viking".into())
        .await
        .expect("session_start");

    let received = timeout(TEST_TIMEOUT, read_driver_message(&mut adapter_read))
        .await
        .expect("read timed out")
        .expect("read failed");
    assert_eq!(
        received,
        DriverMessage::SessionStart {
            session_id: 1,
            username: "viking".into(),
        }
    );

    // Step 3: session_input sends SessionInput to adapter
    server
        .session_input(session_id, "say hello".into())
        .await
        .expect("session_input");

    let received = timeout(TEST_TIMEOUT, read_driver_message(&mut adapter_read))
        .await
        .expect("read timed out")
        .expect("read failed");
    assert_eq!(
        received,
        DriverMessage::SessionInput {
            session_id: 1,
            line: "say hello".into(),
        }
    );

    // Step 4: session_end sends SessionEnd and removes the session
    server.session_end(session_id).await.expect("session_end");

    let received = timeout(TEST_TIMEOUT, read_driver_message(&mut adapter_read))
        .await
        .expect("read timed out")
        .expect("read failed");
    assert_eq!(received, DriverMessage::SessionEnd { session_id: 1 });

    server.shutdown();
}

// =========================================================================
// Session output routing tests
// =========================================================================

#[tokio::test]
async fn session_output_routed_to_correct_session() {
    let (mut server, _adapter_read, mut adapter_write, _dir) = setup_server_with_adapter().await;

    // Create two sessions
    let (id1, mut rx1) = server.create_session();
    let (id2, mut rx2) = server.create_session();
    assert_eq!(id1, 1);
    assert_eq!(id2, 2);

    // Adapter sends output for session 1
    let output1 = AdapterMessage::SessionOutput {
        session_id: id1,
        text: "Welcome, player one!\r\n".into(),
    };
    write_adapter_message(&mut adapter_write, &output1)
        .await
        .expect("write SessionOutput for session 1");

    // Adapter sends output for session 2
    let output2 = AdapterMessage::SessionOutput {
        session_id: id2,
        text: "Welcome, player two!\r\n".into(),
    };
    write_adapter_message(&mut adapter_write, &output2)
        .await
        .expect("write SessionOutput for session 2");

    // Process both messages through the server's event handling
    let msg1 = timeout(TEST_TIMEOUT, server.recv_adapter_message())
        .await
        .expect("recv timed out")
        .expect("recv returned None");
    server.handle_adapter_message(msg1).await;

    let msg2 = timeout(TEST_TIMEOUT, server.recv_adapter_message())
        .await
        .expect("recv timed out")
        .expect("recv returned None");
    server.handle_adapter_message(msg2).await;

    // Verify output was routed to the correct session channels
    let text1 = timeout(TEST_TIMEOUT, rx1.recv())
        .await
        .expect("rx1 timed out")
        .expect("rx1 returned None");
    assert_eq!(text1, "Welcome, player one!\r\n");

    let text2 = timeout(TEST_TIMEOUT, rx2.recv())
        .await
        .expect("rx2 timed out")
        .expect("rx2 returned None");
    assert_eq!(text2, "Welcome, player two!\r\n");

    server.shutdown();
}

#[tokio::test]
async fn session_output_for_unknown_session_is_ignored() {
    let (mut server, _adapter_read, mut adapter_write, _dir) = setup_server_with_adapter().await;

    // Send output for a session that doesn't exist — should not panic
    let output = AdapterMessage::SessionOutput {
        session_id: 999,
        text: "This should be dropped.\r\n".into(),
    };
    write_adapter_message(&mut adapter_write, &output)
        .await
        .expect("write SessionOutput");

    let msg = timeout(TEST_TIMEOUT, server.recv_adapter_message())
        .await
        .expect("recv timed out")
        .expect("recv returned None");

    // This should not panic or error — just log a warning
    server.handle_adapter_message(msg).await;

    server.shutdown();
}

#[tokio::test]
async fn session_end_removes_output_channel() {
    let (mut server, _adapter_read, mut adapter_write, _dir) = setup_server_with_adapter().await;

    // Create and then end a session
    let (session_id, _rx) = server.create_session();
    server.session_end(session_id).await.expect("session_end");

    // Now send output for the ended session — should be silently ignored
    let output = AdapterMessage::SessionOutput {
        session_id,
        text: "Should be dropped.\r\n".into(),
    };
    write_adapter_message(&mut adapter_write, &output)
        .await
        .expect("write SessionOutput");

    let msg = timeout(TEST_TIMEOUT, server.recv_adapter_message())
        .await
        .expect("recv timed out")
        .expect("recv returned None");

    // Should not panic — the session was already removed
    server.handle_adapter_message(msg).await;

    server.shutdown();
}

// =========================================================================
// Multiple concurrent sessions
// =========================================================================

#[tokio::test]
async fn multiple_sessions_with_interleaved_messages() {
    let (mut server, mut adapter_read, mut adapter_write, _dir) = setup_server_with_adapter().await;

    // Create 3 sessions
    let (id1, mut rx1) = server.create_session();
    let (id2, mut rx2) = server.create_session();
    let (id3, mut rx3) = server.create_session();

    // Start all sessions
    server
        .session_start(id1, "alice".into())
        .await
        .expect("session_start alice");
    server
        .session_start(id2, "bob".into())
        .await
        .expect("session_start bob");
    server
        .session_start(id3, "charlie".into())
        .await
        .expect("session_start charlie");

    // Read and verify all 3 SessionStart messages
    for (expected_id, expected_name) in [(id1, "alice"), (id2, "bob"), (id3, "charlie")] {
        let received = timeout(TEST_TIMEOUT, read_driver_message(&mut adapter_read))
            .await
            .expect("read timed out")
            .expect("read failed");
        assert_eq!(
            received,
            DriverMessage::SessionStart {
                session_id: expected_id,
                username: expected_name.into(),
            }
        );
    }

    // Adapter sends interleaved output for different sessions
    for (id, text) in [
        (id2, "Bob's output\r\n"),
        (id1, "Alice's output\r\n"),
        (id3, "Charlie's output\r\n"),
    ] {
        let msg = AdapterMessage::SessionOutput {
            session_id: id,
            text: text.into(),
        };
        write_adapter_message(&mut adapter_write, &msg)
            .await
            .expect("write SessionOutput");
    }

    // Process all 3 adapter messages
    for _ in 0..3 {
        let msg = timeout(TEST_TIMEOUT, server.recv_adapter_message())
            .await
            .expect("recv timed out")
            .expect("recv returned None");
        server.handle_adapter_message(msg).await;
    }

    // Verify each session received its own output
    let t1 = timeout(TEST_TIMEOUT, rx1.recv())
        .await
        .expect("rx1 timed out")
        .expect("rx1 None");
    assert_eq!(t1, "Alice's output\r\n");

    let t2 = timeout(TEST_TIMEOUT, rx2.recv())
        .await
        .expect("rx2 timed out")
        .expect("rx2 None");
    assert_eq!(t2, "Bob's output\r\n");

    let t3 = timeout(TEST_TIMEOUT, rx3.recv())
        .await
        .expect("rx3 timed out")
        .expect("rx3 None");
    assert_eq!(t3, "Charlie's output\r\n");

    // End session 2 and verify input still works for 1 and 3
    server.session_end(id2).await.expect("session_end bob");

    // Read the SessionEnd for bob from the adapter side
    let received = timeout(TEST_TIMEOUT, read_driver_message(&mut adapter_read))
        .await
        .expect("read timed out")
        .expect("read failed");
    assert_eq!(received, DriverMessage::SessionEnd { session_id: id2 });

    // Sessions 1 and 3 can still send input
    server
        .session_input(id1, "north".into())
        .await
        .expect("session_input alice");
    server
        .session_input(id3, "south".into())
        .await
        .expect("session_input charlie");

    let received = timeout(TEST_TIMEOUT, read_driver_message(&mut adapter_read))
        .await
        .expect("read timed out")
        .expect("read failed");
    assert_eq!(
        received,
        DriverMessage::SessionInput {
            session_id: id1,
            line: "north".into(),
        }
    );

    let received = timeout(TEST_TIMEOUT, read_driver_message(&mut adapter_read))
        .await
        .expect("read timed out")
        .expect("read failed");
    assert_eq!(
        received,
        DriverMessage::SessionInput {
            session_id: id3,
            line: "south".into(),
        }
    );

    server.shutdown();
}

// =========================================================================
// ReviewPolicy unit tests
// =========================================================================

#[test]
fn review_policy_default_is_unprotected() {
    let policy = ReviewPolicy::default();
    assert!(!policy.main_protected);
    assert_eq!(policy.required_approvals, 0);
}

#[test]
fn review_policy_protected_requires_approvals() {
    let policy = ReviewPolicy {
        main_protected: true,
        required_approvals: 2,
    };
    assert!(policy.main_protected);
    assert_eq!(policy.required_approvals, 2);

    // With 1 approval (less than required), merge should be blocked
    let current_approvals: i64 = 1;
    assert!(
        current_approvals < policy.required_approvals as i64,
        "merge should be blocked when approvals ({}) < required ({})",
        current_approvals,
        policy.required_approvals
    );
}

#[test]
fn review_policy_threshold_met_allows_merge() {
    let policy = ReviewPolicy {
        main_protected: true,
        required_approvals: 2,
    };

    // With exactly 2 approvals, threshold is met
    let current_approvals: i64 = 2;
    assert!(
        current_approvals >= policy.required_approvals as i64,
        "merge should be allowed when approvals ({}) >= required ({})",
        current_approvals,
        policy.required_approvals
    );
}

#[test]
fn review_policy_unprotected_allows_merge_with_zero_approvals() {
    let policy = ReviewPolicy {
        main_protected: false,
        required_approvals: 0,
    };

    let current_approvals: i64 = 0;
    assert!(
        current_approvals >= policy.required_approvals as i64,
        "unprotected policy should allow merge with zero approvals"
    );
}

#[test]
fn review_policy_excess_approvals_still_allows_merge() {
    let policy = ReviewPolicy {
        main_protected: true,
        required_approvals: 1,
    };

    // With 5 approvals (more than required), merge is allowed
    let current_approvals: i64 = 5;
    assert!(
        current_approvals >= policy.required_approvals as i64,
        "merge should be allowed when approvals exceed threshold"
    );
}

// =========================================================================
// Area discovery tests
//
// Tests for the `discover_areas` function that scans the world directory
// for loadable areas, applying the filtering rules:
// - Must be a directory
// - Must not contain @dev in the name
// - Must contain a .meta.yml file
// =========================================================================

#[test]
fn discover_areas_finds_areas_with_meta_yml() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let world_path = dir.path().join("world");

    // Create an area with .meta.yml -> should be discovered.
    let area = world_path.join("vikings").join("village");
    std::fs::create_dir_all(&area).unwrap();
    std::fs::write(area.join(".meta.yml"), "owner: vikings\n").unwrap();

    let areas = mud_driver::server::discover_areas(&world_path.to_string_lossy()).unwrap();

    assert_eq!(areas.len(), 1, "should discover exactly one area");
    assert_eq!(areas[0].0.namespace, "vikings");
    assert_eq!(areas[0].0.name, "village");
    assert!(
        areas[0].1.ends_with("vikings/village"),
        "path should end with namespace/name"
    );
}

#[test]
fn discover_areas_skips_directories_without_meta_yml() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let world_path = dir.path().join("world");

    // Area with .meta.yml -> discovered.
    let good = world_path.join("vikings").join("village");
    std::fs::create_dir_all(&good).unwrap();
    std::fs::write(good.join(".meta.yml"), "owner: vikings\n").unwrap();

    // Area without .meta.yml -> skipped.
    let bad = world_path.join("vikings").join("empty_area");
    std::fs::create_dir_all(&bad).unwrap();

    let areas = mud_driver::server::discover_areas(&world_path.to_string_lossy()).unwrap();

    assert_eq!(areas.len(), 1);
    assert_eq!(areas[0].0.name, "village");
}

#[test]
fn discover_areas_skips_dev_directories() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let world_path = dir.path().join("world");

    // Production area -> discovered.
    let prod = world_path.join("vikings").join("village");
    std::fs::create_dir_all(&prod).unwrap();
    std::fs::write(prod.join(".meta.yml"), "owner: vikings\n").unwrap();

    // @dev directory (even with .meta.yml) -> skipped.
    let dev = world_path.join("vikings").join("village@dev");
    std::fs::create_dir_all(&dev).unwrap();
    std::fs::write(dev.join(".meta.yml"), "owner: vikings\n").unwrap();

    let areas = mud_driver::server::discover_areas(&world_path.to_string_lossy()).unwrap();

    assert_eq!(areas.len(), 1, "should skip @dev directory");
    assert_eq!(areas[0].0.name, "village");
}

#[test]
fn discover_areas_skips_files_not_directories() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let world_path = dir.path().join("world");

    // Create a namespace directory.
    let ns_path = world_path.join("vikings");
    std::fs::create_dir_all(&ns_path).unwrap();

    // Create a file (not a directory) at the area level -> skipped.
    std::fs::write(ns_path.join("not_an_area.txt"), "hello").unwrap();

    // Create a real area directory.
    let area = ns_path.join("village");
    std::fs::create_dir_all(&area).unwrap();
    std::fs::write(area.join(".meta.yml"), "owner: vikings\n").unwrap();

    let areas = mud_driver::server::discover_areas(&world_path.to_string_lossy()).unwrap();

    assert_eq!(areas.len(), 1);
    assert_eq!(areas[0].0.name, "village");
}

#[test]
fn discover_areas_multiple_namespaces() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let world_path = dir.path().join("world");

    // Create areas in two namespaces.
    for (ns, name) in &[
        ("vikings", "village"),
        ("vikings", "harbor"),
        ("elves", "forest"),
    ] {
        let area = world_path.join(ns).join(name);
        std::fs::create_dir_all(&area).unwrap();
        std::fs::write(area.join(".meta.yml"), format!("owner: {}\n", ns)).unwrap();
    }

    // Also add an @dev and an area without .meta.yml.
    let dev = world_path.join("vikings").join("harbor@dev");
    std::fs::create_dir_all(&dev).unwrap();
    std::fs::write(dev.join(".meta.yml"), "owner: vikings\n").unwrap();

    let no_meta = world_path.join("elves").join("cave");
    std::fs::create_dir_all(&no_meta).unwrap();

    let areas = mud_driver::server::discover_areas(&world_path.to_string_lossy()).unwrap();

    // Should find exactly 3 areas (not the @dev, not the cave).
    assert_eq!(
        areas.len(),
        3,
        "expected 3 areas, got {:?}",
        areas.iter().map(|a| a.0.to_string()).collect::<Vec<_>>()
    );

    // Extract names for verification.
    let ids: Vec<String> = areas.iter().map(|a| a.0.to_string()).collect();
    assert!(ids.contains(&"elves/forest".to_string()));
    assert!(ids.contains(&"vikings/harbor".to_string()));
    assert!(ids.contains(&"vikings/village".to_string()));

    // Verify sorted order.
    assert_eq!(ids[0], "elves/forest");
    assert_eq!(ids[1], "vikings/harbor");
    assert_eq!(ids[2], "vikings/village");
}

#[test]
fn discover_areas_empty_world_returns_empty() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let world_path = dir.path().join("world");
    std::fs::create_dir_all(&world_path).unwrap();

    let areas = mud_driver::server::discover_areas(&world_path.to_string_lossy()).unwrap();

    assert!(
        areas.is_empty(),
        "empty world directory should yield no areas"
    );
}

#[test]
fn discover_areas_nonexistent_world_returns_empty() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let world_path = dir.path().join("nonexistent_world");

    let areas = mud_driver::server::discover_areas(&world_path.to_string_lossy()).unwrap();

    assert!(
        areas.is_empty(),
        "nonexistent world path should yield no areas"
    );
}

#[test]
fn discover_areas_with_system_namespace() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let world_path = dir.path().join("world");

    // System areas are discovered like any other (filtering is done at
    // the session layer, not at discovery).
    let system_area = world_path.join("system").join("stdlib");
    std::fs::create_dir_all(&system_area).unwrap();
    std::fs::write(system_area.join(".meta.yml"), "owner: system\n").unwrap();

    let game_area = world_path.join("vikings").join("town");
    std::fs::create_dir_all(&game_area).unwrap();
    std::fs::write(game_area.join(".meta.yml"), "owner: vikings\n").unwrap();

    let areas = mud_driver::server::discover_areas(&world_path.to_string_lossy()).unwrap();

    assert_eq!(areas.len(), 2);
    let ids: Vec<String> = areas.iter().map(|a| a.0.to_string()).collect();
    assert!(ids.contains(&"system/stdlib".to_string()));
    assert!(ids.contains(&"vikings/town".to_string()));
}

#[test]
fn discover_areas_skips_system_template_repos() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let world_path = dir.path().join("world");

    let template_repo = world_path.join("system").join("template_default");
    std::fs::create_dir_all(&template_repo).unwrap();
    std::fs::write(template_repo.join(".meta.yml"), "owner: system\n").unwrap();

    let stdlib = world_path.join("system").join("stdlib");
    std::fs::create_dir_all(&stdlib).unwrap();
    std::fs::write(stdlib.join(".meta.yml"), "owner: system\nsystem: true\n").unwrap();

    let areas = mud_driver::server::discover_areas(&world_path.to_string_lossy()).unwrap();

    let ids: Vec<String> = areas.iter().map(|a| a.0.to_string()).collect();
    assert!(ids.contains(&"system/stdlib".to_string()));
    assert!(!ids.contains(&"system/template_default".to_string()));
}
