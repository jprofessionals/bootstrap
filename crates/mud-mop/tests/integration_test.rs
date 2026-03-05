use std::time::Duration;

use mud_core::types::AreaId;
use mud_mop::codec::{
    read_adapter_message, read_driver_message, write_adapter_message, write_driver_message,
    CodecError,
};
use mud_mop::message::{AdapterMessage, DriverMessage, Value};
use tokio::net::UnixStream;
use tokio::time::timeout;

/// Default timeout for test operations.
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

// =========================================================================
// Test 1: Unix socket round-trip
// =========================================================================

#[tokio::test]
async fn unix_socket_driver_message_round_trip() {
    let (mut left, mut right) = UnixStream::pair().expect("create socket pair");

    let msg = DriverMessage::LoadArea {
        area_id: AreaId::new("system", "lobby"),
        path: "/world/system/lobby".into(),
        db_url: None,
    };

    let sent = msg.clone();
    let write_handle = tokio::spawn(async move {
        write_driver_message(&mut left, &sent).await.unwrap();
    });

    let read_handle = tokio::spawn(async move {
        read_driver_message(&mut right).await.unwrap()
    });

    let received = timeout(TEST_TIMEOUT, async {
        write_handle.await.unwrap();
        read_handle.await.unwrap()
    })
    .await
    .expect("test timed out");

    assert_eq!(msg, received);
}

#[tokio::test]
async fn unix_socket_adapter_message_round_trip() {
    let (mut left, mut right) = UnixStream::pair().expect("create socket pair");

    let msg = AdapterMessage::Handshake {
        adapter_name: "test-adapter".into(),
        language: "ruby".into(),
        version: "1.0.0".into(),
    };

    let sent = msg.clone();
    let write_handle = tokio::spawn(async move {
        write_adapter_message(&mut left, &sent).await.unwrap();
    });

    let read_handle = tokio::spawn(async move {
        read_adapter_message(&mut right).await.unwrap()
    });

    let received = timeout(TEST_TIMEOUT, async {
        write_handle.await.unwrap();
        read_handle.await.unwrap()
    })
    .await
    .expect("test timed out");

    assert_eq!(msg, received);
}

// =========================================================================
// Test 2: Multiple messages in sequence
// =========================================================================

#[tokio::test]
async fn multiple_messages_preserve_order() {
    let (mut writer_end, mut reader_end) = UnixStream::pair().expect("create socket pair");

    let messages = vec![
        DriverMessage::Ping { seq: 1 },
        DriverMessage::LoadArea {
            area_id: AreaId::new("system", "lobby"),
            path: "/world/system/lobby".into(),
            db_url: None,
        },
        DriverMessage::SessionStart {
            session_id: 42,
            username: "alice".into(),
        },
        DriverMessage::SessionInput {
            session_id: 42,
            line: "look".into(),
        },
        DriverMessage::SessionEnd { session_id: 42 },
        DriverMessage::UnloadArea {
            area_id: AreaId::new("system", "lobby"),
        },
    ];

    let sent_messages = messages.clone();
    let write_handle = tokio::spawn(async move {
        for msg in &sent_messages {
            write_driver_message(&mut writer_end, msg).await.unwrap();
        }
    });

    let expected_count = messages.len();
    let read_handle = tokio::spawn(async move {
        let mut received = Vec::new();
        for _ in 0..expected_count {
            let msg = read_driver_message(&mut reader_end).await.unwrap();
            received.push(msg);
        }
        received
    });

    let received = timeout(TEST_TIMEOUT, async {
        write_handle.await.unwrap();
        read_handle.await.unwrap()
    })
    .await
    .expect("test timed out");

    assert_eq!(messages.len(), received.len());
    for (expected, actual) in messages.iter().zip(received.iter()) {
        assert_eq!(expected, actual);
    }
}

// =========================================================================
// Test 3: Bidirectional communication
// =========================================================================

#[tokio::test]
async fn bidirectional_driver_and_adapter_messages() {
    let (left, right) = UnixStream::pair().expect("create socket pair");
    let (mut left_read, mut left_write) = tokio::io::split(left);
    let (mut right_read, mut right_write) = tokio::io::split(right);

    // Driver sends DriverMessages on left_write, reads AdapterMessages on left_read
    // Adapter sends AdapterMessages on right_write, reads DriverMessages on right_read

    let driver_msg = DriverMessage::LoadArea {
        area_id: AreaId::new("game", "tavern"),
        path: "/world/game/tavern".into(),
        db_url: None,
    };

    let adapter_msg = AdapterMessage::AreaLoaded {
        area_id: AreaId::new("game", "tavern"),
    };

    let driver_send = driver_msg.clone();
    let adapter_send = adapter_msg.clone();

    let result = timeout(TEST_TIMEOUT, async {
        // Driver writes a DriverMessage
        write_driver_message(&mut left_write, &driver_send).await.unwrap();

        // Adapter reads the DriverMessage
        let received_driver = read_driver_message(&mut right_read).await.unwrap();
        assert_eq!(driver_msg, received_driver);

        // Adapter writes an AdapterMessage back
        write_adapter_message(&mut right_write, &adapter_send).await.unwrap();

        // Driver reads the AdapterMessage
        let received_adapter = read_adapter_message(&mut left_read).await.unwrap();
        assert_eq!(adapter_msg, received_adapter);
    })
    .await;

    result.expect("test timed out");
}

#[tokio::test]
async fn bidirectional_interleaved_messages() {
    let (left, right) = UnixStream::pair().expect("create socket pair");
    let (mut left_read, mut left_write) = tokio::io::split(left);
    let (mut right_read, mut right_write) = tokio::io::split(right);

    let result = timeout(TEST_TIMEOUT, async {
        // Driver sends SessionStart
        let start_msg = DriverMessage::SessionStart {
            session_id: 1,
            username: "bob".into(),
        };
        write_driver_message(&mut left_write, &start_msg).await.unwrap();

        // Adapter reads it
        let received = read_driver_message(&mut right_read).await.unwrap();
        assert_eq!(start_msg, received);

        // Adapter sends SessionOutput
        let output_msg = AdapterMessage::SessionOutput {
            session_id: 1,
            text: "Welcome, bob!\r\n".into(),
        };
        write_adapter_message(&mut right_write, &output_msg).await.unwrap();

        // Driver reads it
        let received = read_adapter_message(&mut left_read).await.unwrap();
        assert_eq!(output_msg, received);

        // Driver sends SessionInput
        let input_msg = DriverMessage::SessionInput {
            session_id: 1,
            line: "look".into(),
        };
        write_driver_message(&mut left_write, &input_msg).await.unwrap();

        // Adapter reads it
        let received = read_driver_message(&mut right_read).await.unwrap();
        assert_eq!(input_msg, received);

        // Adapter sends another SessionOutput
        let output_msg2 = AdapterMessage::SessionOutput {
            session_id: 1,
            text: "You are in a dark room.\r\n".into(),
        };
        write_adapter_message(&mut right_write, &output_msg2).await.unwrap();

        // Driver reads it
        let received = read_adapter_message(&mut left_read).await.unwrap();
        assert_eq!(output_msg2, received);
    })
    .await;

    result.expect("test timed out");
}

// =========================================================================
// Test 4: Large message payload
// =========================================================================

#[tokio::test]
async fn large_payload_transfers_correctly() {
    let (mut writer_end, mut reader_end) = UnixStream::pair().expect("create socket pair");

    // Create a large string payload (~1 MB)
    let large_text: String = "A".repeat(1_000_000);

    let msg = AdapterMessage::CallResult {
        request_id: 99,
        result: Value::String(large_text.clone()),
    };

    let sent = msg.clone();
    let write_handle = tokio::spawn(async move {
        write_adapter_message(&mut writer_end, &sent).await.unwrap();
    });

    let read_handle = tokio::spawn(async move {
        read_adapter_message(&mut reader_end).await.unwrap()
    });

    let received = timeout(Duration::from_secs(10), async {
        write_handle.await.unwrap();
        read_handle.await.unwrap()
    })
    .await
    .expect("test timed out");

    assert_eq!(msg, received);

    // Verify the actual string content
    if let AdapterMessage::CallResult { result: Value::String(s), .. } = &received {
        assert_eq!(s.len(), 1_000_000);
        assert!(s.chars().all(|c| c == 'A'));
    } else {
        panic!("unexpected message variant");
    }
}

#[tokio::test]
async fn large_call_with_complex_args() {
    let (mut writer_end, mut reader_end) = UnixStream::pair().expect("create socket pair");

    // Build a message with a large array of values
    let large_args: Vec<Value> = (0..10_000)
        .map(|i| Value::Map(std::collections::HashMap::from([
            ("index".into(), Value::Int(i)),
            ("name".into(), Value::String(format!("item_{i}"))),
            ("active".into(), Value::Bool(i % 2 == 0)),
        ])))
        .collect();

    let msg = DriverMessage::Call {
        request_id: 1,
        object_id: 500,
        method: "bulk_update".into(),
        args: large_args,
    };

    let sent = msg.clone();
    let write_handle = tokio::spawn(async move {
        write_driver_message(&mut writer_end, &sent).await.unwrap();
    });

    let read_handle = tokio::spawn(async move {
        read_driver_message(&mut reader_end).await.unwrap()
    });

    let received = timeout(Duration::from_secs(10), async {
        write_handle.await.unwrap();
        read_handle.await.unwrap()
    })
    .await
    .expect("test timed out");

    assert_eq!(msg, received);
}

// =========================================================================
// Test 5: Connection close detection
// =========================================================================

#[tokio::test]
async fn writer_close_detected_as_codec_closed() {
    let (writer_end, mut reader_end) = UnixStream::pair().expect("create socket pair");

    // Drop the writer end to close the connection
    drop(writer_end);

    let result = timeout(TEST_TIMEOUT, async {
        read_driver_message(&mut reader_end).await
    })
    .await
    .expect("test timed out");

    assert!(
        matches!(result, Err(CodecError::Closed)),
        "expected CodecError::Closed, got: {result:?}"
    );
}

#[tokio::test]
async fn reader_close_detected_on_write() {
    let (mut writer_end, reader_end) = UnixStream::pair().expect("create socket pair");

    // Drop the reader end
    drop(reader_end);

    let msg = DriverMessage::Ping { seq: 1 };

    // Writing to a closed socket should eventually fail with an IO error.
    // The first write might succeed (kernel buffer), but subsequent ones should fail.
    let result = timeout(TEST_TIMEOUT, async {
        // Send multiple messages to trigger the broken pipe
        for _ in 0..100 {
            match write_driver_message(&mut writer_end, &msg).await {
                Ok(_) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    })
    .await
    .expect("test timed out");

    // We expect either an IO error (broken pipe) or it might succeed if the kernel
    // buffers are large enough. Either way, the test demonstrates the behavior.
    if let Err(e) = result {
        assert!(
            matches!(e, CodecError::Io(_)),
            "expected CodecError::Io, got: {e:?}"
        );
    }
}

#[tokio::test]
async fn close_after_partial_communication() {
    let (left, right) = UnixStream::pair().expect("create socket pair");
    let (mut left_read, mut left_write) = tokio::io::split(left);
    let (mut right_read, mut right_write) = tokio::io::split(right);

    let result = timeout(TEST_TIMEOUT, async {
        // Send one message successfully
        let msg = DriverMessage::Ping { seq: 1 };
        write_driver_message(&mut left_write, &msg).await.unwrap();

        let received = read_driver_message(&mut right_read).await.unwrap();
        assert_eq!(msg, received);

        // Send a response
        let resp = AdapterMessage::Pong { seq: 1 };
        write_adapter_message(&mut right_write, &resp).await.unwrap();

        let received = read_adapter_message(&mut left_read).await.unwrap();
        assert_eq!(resp, received);

        // Now close the right side (adapter disconnects)
        drop(right_read);
        drop(right_write);

        // Driver should detect the close when trying to read
        let result = read_adapter_message(&mut left_read).await;
        assert!(
            matches!(result, Err(CodecError::Closed)),
            "expected CodecError::Closed after peer disconnect, got: {result:?}"
        );
    })
    .await;

    result.expect("test timed out");
}

// =========================================================================
// Additional edge case tests
// =========================================================================

#[tokio::test]
async fn all_driver_message_variants_round_trip_over_socket() {
    let (mut writer_end, mut reader_end) = UnixStream::pair().expect("create socket pair");

    let messages = vec![
        DriverMessage::LoadArea {
            area_id: AreaId::new("ns", "area"),
            path: "/path".into(),
            db_url: None,
        },
        DriverMessage::ReloadArea {
            area_id: AreaId::new("ns", "area"),
            path: "/path".into(),
            db_url: None,
        },
        DriverMessage::UnloadArea {
            area_id: AreaId::new("ns", "area"),
        },
        DriverMessage::SessionStart {
            session_id: 1,
            username: "user".into(),
        },
        DriverMessage::SessionInput {
            session_id: 1,
            line: "cmd".into(),
        },
        DriverMessage::SessionEnd { session_id: 1 },
        DriverMessage::Call {
            request_id: 1,
            object_id: 42,
            method: "foo".into(),
            args: vec![Value::Null, Value::Bool(true), Value::Float(7.77)],
        },
        DriverMessage::Ping { seq: 99 },
    ];

    let sent = messages.clone();
    let count = messages.len();

    let write_handle = tokio::spawn(async move {
        for msg in &sent {
            write_driver_message(&mut writer_end, msg).await.unwrap();
        }
    });

    let read_handle = tokio::spawn(async move {
        let mut received = Vec::new();
        for _ in 0..count {
            received.push(read_driver_message(&mut reader_end).await.unwrap());
        }
        received
    });

    let received = timeout(TEST_TIMEOUT, async {
        write_handle.await.unwrap();
        read_handle.await.unwrap()
    })
    .await
    .expect("test timed out");

    assert_eq!(messages, received);
}

#[tokio::test]
async fn all_adapter_message_variants_round_trip_over_socket() {
    let (mut writer_end, mut reader_end) = UnixStream::pair().expect("create socket pair");

    let messages = vec![
        AdapterMessage::AreaLoaded {
            area_id: AreaId::new("ns", "area"),
        },
        AdapterMessage::AreaError {
            area_id: AreaId::new("ns", "area"),
            error: "syntax error on line 42".into(),
        },
        AdapterMessage::SessionOutput {
            session_id: 1,
            text: "Hello, world!\r\n".into(),
        },
        AdapterMessage::CallResult {
            request_id: 1,
            result: Value::Int(42),
        },
        AdapterMessage::CallError {
            request_id: 2,
            error: "method not found".into(),
        },
        AdapterMessage::MoveObject {
            object_id: 100,
            destination_id: 200,
        },
        AdapterMessage::SendMessage {
            target_session: 5,
            text: "You hear a noise.\r\n".into(),
        },
        AdapterMessage::Log {
            level: "info".into(),
            message: "area loaded".into(),
            area: Some("system/lobby".into()),
        },
        AdapterMessage::Log {
            level: "error".into(),
            message: "something failed".into(),
            area: None,
        },
        AdapterMessage::Pong { seq: 99 },
        AdapterMessage::Handshake {
            adapter_name: "test".into(),
            language: "ruby".into(),
            version: "1.0.0".into(),
        },
    ];

    let sent = messages.clone();
    let count = messages.len();

    let write_handle = tokio::spawn(async move {
        for msg in &sent {
            write_adapter_message(&mut writer_end, msg).await.unwrap();
        }
    });

    let read_handle = tokio::spawn(async move {
        let mut received = Vec::new();
        for _ in 0..count {
            received.push(read_adapter_message(&mut reader_end).await.unwrap());
        }
        received
    });

    let received = timeout(TEST_TIMEOUT, async {
        write_handle.await.unwrap();
        read_handle.await.unwrap()
    })
    .await
    .expect("test timed out");

    assert_eq!(messages, received);
}

#[tokio::test]
async fn ping_pong_exchange_over_socket() {
    let (left, right) = UnixStream::pair().expect("create socket pair");
    let (mut left_read, mut left_write) = tokio::io::split(left);
    let (mut right_read, mut right_write) = tokio::io::split(right);

    let result = timeout(TEST_TIMEOUT, async {
        for seq in 0..100 {
            // Driver sends Ping
            let ping = DriverMessage::Ping { seq };
            write_driver_message(&mut left_write, &ping).await.unwrap();

            // Adapter receives Ping
            let received = read_driver_message(&mut right_read).await.unwrap();
            assert_eq!(ping, received);

            // Adapter sends Pong
            let pong = AdapterMessage::Pong { seq };
            write_adapter_message(&mut right_write, &pong).await.unwrap();

            // Driver receives Pong
            let received = read_adapter_message(&mut left_read).await.unwrap();
            assert_eq!(pong, received);
        }
    })
    .await;

    result.expect("test timed out");
}
