use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::message::{AdapterMessage, DriverMessage};

/// Maximum allowed message size (16 MB).
pub const MAX_MESSAGE_SIZE: u32 = 16 * 1024 * 1024;

/// Errors that can occur during MOP message encoding/decoding.
#[derive(thiserror::Error, Debug)]
pub enum CodecError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("Deserialization error: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    #[error("Message too large: {size} bytes (max {max})")]
    TooLarge { size: u32, max: u32 },
    #[error("Connection closed")]
    Closed,
}

/// Write a length-prefixed frame to the writer.
///
/// Wire format: `[4 bytes: big-endian u32 length][N bytes: payload]`
async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    payload: &[u8],
) -> Result<(), CodecError> {
    let len = payload.len() as u32;
    if len > MAX_MESSAGE_SIZE {
        return Err(CodecError::TooLarge {
            size: len,
            max: MAX_MESSAGE_SIZE,
        });
    }
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(payload).await?;
    writer.flush().await?;
    Ok(())
}

/// Read a length-prefixed frame from the reader.
///
/// Returns `CodecError::Closed` on clean EOF (zero bytes read for the length
/// prefix) and `CodecError::TooLarge` if the declared size exceeds
/// `MAX_MESSAGE_SIZE`.
async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>, CodecError> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(CodecError::Closed);
        }
        Err(e) => return Err(CodecError::Io(e)),
    }

    let len = u32::from_be_bytes(len_buf);
    if len > MAX_MESSAGE_SIZE {
        return Err(CodecError::TooLarge {
            size: len,
            max: MAX_MESSAGE_SIZE,
        });
    }

    let mut payload = vec![0u8; len as usize];
    reader.read_exact(&mut payload).await?;
    Ok(payload)
}

/// Serialize a [`DriverMessage`] and write it as a length-prefixed MessagePack
/// frame.
pub async fn write_driver_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    msg: &DriverMessage,
) -> Result<(), CodecError> {
    let payload = rmp_serde::to_vec_named(msg)?;
    write_frame(writer, &payload).await
}

/// Serialize an [`AdapterMessage`] and write it as a length-prefixed
/// MessagePack frame.
pub async fn write_adapter_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    msg: &AdapterMessage,
) -> Result<(), CodecError> {
    let payload = rmp_serde::to_vec_named(msg)?;
    write_frame(writer, &payload).await
}

/// Read a length-prefixed MessagePack frame and deserialize it into a
/// [`DriverMessage`].
pub async fn read_driver_message<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<DriverMessage, CodecError> {
    let payload = read_frame(reader).await?;
    let msg = rmp_serde::from_slice(&payload)?;
    Ok(msg)
}

/// Read a length-prefixed MessagePack frame and deserialize it into an
/// [`AdapterMessage`].
pub async fn read_adapter_message<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<AdapterMessage, CodecError> {
    let payload = read_frame(reader).await?;
    let msg = rmp_serde::from_slice(&payload)?;
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Value;
    use mud_core::types::AreaId;

    #[tokio::test]
    async fn driver_message_round_trip() {
        let msg = DriverMessage::Ping { seq: 42 };

        let mut buf = Vec::new();
        write_driver_message(&mut buf, &msg).await.unwrap();

        let mut cursor = &buf[..];
        let decoded = read_driver_message(&mut cursor).await.unwrap();
        assert_eq!(msg, decoded);
    }

    #[tokio::test]
    async fn adapter_message_round_trip() {
        let msg = AdapterMessage::Handshake {
            adapter_name: "test".into(),
            language: "ruby".into(),
            version: "0.1.0".into(),
            languages: vec![],
        };

        let mut buf = Vec::new();
        write_adapter_message(&mut buf, &msg).await.unwrap();

        let mut cursor = &buf[..];
        let decoded = read_adapter_message(&mut cursor).await.unwrap();
        assert_eq!(msg, decoded);
    }

    #[tokio::test]
    async fn eof_returns_closed() {
        let mut cursor: &[u8] = &[];
        let result = read_driver_message(&mut cursor).await;
        assert!(matches!(result, Err(CodecError::Closed)));
    }

    #[tokio::test]
    async fn too_large_message_rejected_on_read() {
        let fake_len: u32 = MAX_MESSAGE_SIZE + 1;
        let mut buf = fake_len.to_be_bytes().to_vec();
        buf.extend_from_slice(&[0u8; 8]); // some trailing bytes

        let mut cursor = &buf[..];
        let result = read_driver_message(&mut cursor).await;
        assert!(matches!(result, Err(CodecError::TooLarge { .. })));
    }

    #[tokio::test]
    async fn length_prefix_is_big_endian() {
        let msg = DriverMessage::Ping { seq: 1 };
        let mut buf = Vec::new();
        write_driver_message(&mut buf, &msg).await.unwrap();

        let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        assert_eq!(len as usize, buf.len() - 4);
    }

    #[tokio::test]
    async fn complex_message_round_trip() {
        let msg = DriverMessage::LoadArea {
            area_id: AreaId::new("system", "lobby"),
            path: "/world/system/lobby".into(),
            db_url: None,
        };

        let mut buf = Vec::new();
        write_driver_message(&mut buf, &msg).await.unwrap();

        let mut cursor = &buf[..];
        let decoded = read_driver_message(&mut cursor).await.unwrap();
        assert_eq!(msg, decoded);
    }

    #[tokio::test]
    async fn call_with_values_round_trip() {
        let msg = DriverMessage::Call {
            request_id: 7,
            object_id: 100,
            method: "attack".into(),
            args: vec![Value::String("troll".into()), Value::Int(5)],
        };

        let mut buf = Vec::new();
        write_driver_message(&mut buf, &msg).await.unwrap();

        let mut cursor = &buf[..];
        let decoded = read_driver_message(&mut cursor).await.unwrap();
        assert_eq!(msg, decoded);
    }

    #[tokio::test]
    async fn multiple_messages_in_stream() {
        let msg1 = DriverMessage::Ping { seq: 1 };
        let msg2 = DriverMessage::Ping { seq: 2 };
        let msg3 = DriverMessage::Ping { seq: 3 };

        let mut buf = Vec::new();
        write_driver_message(&mut buf, &msg1).await.unwrap();
        write_driver_message(&mut buf, &msg2).await.unwrap();
        write_driver_message(&mut buf, &msg3).await.unwrap();

        let mut cursor = &buf[..];
        let d1 = read_driver_message(&mut cursor).await.unwrap();
        let d2 = read_driver_message(&mut cursor).await.unwrap();
        let d3 = read_driver_message(&mut cursor).await.unwrap();
        assert_eq!(d1, msg1);
        assert_eq!(d2, msg2);
        assert_eq!(d3, msg3);

        // After all messages, should get Closed
        let result = read_driver_message(&mut cursor).await;
        assert!(matches!(result, Err(CodecError::Closed)));
    }

    #[tokio::test]
    async fn adapter_session_output_round_trip() {
        let msg = AdapterMessage::SessionOutput {
            session_id: 5,
            text: "You see a sword.\n".into(),
        };

        let mut buf = Vec::new();
        write_adapter_message(&mut buf, &msg).await.unwrap();

        let mut cursor = &buf[..];
        let decoded = read_adapter_message(&mut cursor).await.unwrap();
        assert_eq!(msg, decoded);
    }

    #[tokio::test]
    async fn adapter_pong_round_trip() {
        let msg = AdapterMessage::Pong { seq: 42 };

        let mut buf = Vec::new();
        write_adapter_message(&mut buf, &msg).await.unwrap();

        let mut cursor = &buf[..];
        let decoded = read_adapter_message(&mut cursor).await.unwrap();
        assert_eq!(msg, decoded);
    }

    #[tokio::test]
    async fn truncated_length_prefix_returns_closed() {
        // Only 2 bytes of a 4-byte length prefix
        let buf = [0u8, 1];
        let mut cursor = &buf[..];
        let result = read_driver_message(&mut cursor).await;
        assert!(matches!(result, Err(CodecError::Closed)));
    }

    #[tokio::test]
    async fn truncated_payload_returns_io_error() {
        // Length prefix says 100 bytes but only 5 bytes follow
        let mut buf = 100u32.to_be_bytes().to_vec();
        buf.extend_from_slice(&[0u8; 5]);
        let mut cursor = &buf[..];
        let result = read_driver_message(&mut cursor).await;
        assert!(matches!(result, Err(CodecError::Io(_))));
    }

    #[tokio::test]
    async fn max_message_size_constant() {
        assert_eq!(MAX_MESSAGE_SIZE, 16 * 1024 * 1024);
    }

    #[tokio::test]
    async fn codec_error_display() {
        let err = CodecError::Closed;
        assert_eq!(format!("{}", err), "Connection closed");

        let err = CodecError::TooLarge {
            size: 100,
            max: 50,
        };
        let display = format!("{}", err);
        assert!(display.contains("100"));
        assert!(display.contains("50"));
    }

    #[tokio::test]
    async fn driver_request_response_round_trip() {
        let msg = DriverMessage::RequestResponse {
            request_id: 99,
            result: Value::String("ok".into()),
        };

        let mut buf = Vec::new();
        write_driver_message(&mut buf, &msg).await.unwrap();

        let mut cursor = &buf[..];
        let decoded = read_driver_message(&mut cursor).await.unwrap();
        assert_eq!(msg, decoded);
    }

    #[tokio::test]
    async fn driver_request_error_round_trip() {
        let msg = DriverMessage::RequestError {
            request_id: 100,
            error: "something broke".into(),
        };

        let mut buf = Vec::new();
        write_driver_message(&mut buf, &msg).await.unwrap();

        let mut cursor = &buf[..];
        let decoded = read_driver_message(&mut cursor).await.unwrap();
        assert_eq!(msg, decoded);
    }

    #[tokio::test]
    async fn adapter_driver_request_round_trip() {
        let msg = AdapterMessage::DriverRequest {
            request_id: 1,
            action: "get_area_info".into(),
            params: Value::Map(std::collections::HashMap::from([
                ("ns".into(), Value::String("game".into())),
            ])),
        };

        let mut buf = Vec::new();
        write_adapter_message(&mut buf, &msg).await.unwrap();

        let mut cursor = &buf[..];
        let decoded = read_adapter_message(&mut cursor).await.unwrap();
        assert_eq!(msg, decoded);
    }
}
