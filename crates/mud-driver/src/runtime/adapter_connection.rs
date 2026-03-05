use anyhow::Result;
use mud_mop::codec::{read_adapter_message, write_driver_message, CodecError};
use mud_mop::message::{AdapterMessage, DriverMessage};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

/// Manages bidirectional communication with a single language adapter over a
/// Unix socket connection.
#[allow(dead_code)]
pub struct AdapterConnection {
    tx: mpsc::Sender<DriverMessage>,
    pub name: String,
    pub language: String,
}

impl AdapterConnection {
    /// Spawn read and write loops for a connected adapter.
    ///
    /// Returns the connection handle (for sending messages to the adapter) and
    /// a receiver that yields messages coming from the adapter.
    pub fn spawn(
        reader: OwnedReadHalf,
        writer: OwnedWriteHalf,
        name: String,
        language: String,
    ) -> (Self, mpsc::Receiver<AdapterMessage>) {
        // Channel for driver → adapter messages (outgoing)
        let (outgoing_tx, outgoing_rx) = mpsc::channel::<DriverMessage>(256);

        // Channel for adapter → driver messages (incoming)
        let (incoming_tx, incoming_rx) = mpsc::channel::<AdapterMessage>(256);

        let lang_write = language.clone();
        let lang_read = language.clone();

        // Write loop: drains outgoing channel and writes frames to the socket.
        tokio::spawn(async move {
            Self::write_loop(writer, outgoing_rx, &lang_write).await;
        });

        // Read loop: reads frames from the socket and sends them to the
        // incoming channel.
        tokio::spawn(async move {
            Self::read_loop(reader, incoming_tx, &lang_read).await;
        });

        let conn = Self {
            tx: outgoing_tx,
            name,
            language,
        };
        (conn, incoming_rx)
    }

    /// Send a message to this adapter.
    pub async fn send(&self, msg: DriverMessage) -> Result<()> {
        self.tx
            .send(msg)
            .await
            .map_err(|_| anyhow::anyhow!("adapter channel closed for {}", self.language))?;
        Ok(())
    }

    /// Write loop: receives messages from the outgoing channel and writes
    /// them as length-prefixed MessagePack frames to the socket.
    async fn write_loop(
        mut writer: OwnedWriteHalf,
        mut rx: mpsc::Receiver<DriverMessage>,
        language: &str,
    ) {
        while let Some(msg) = rx.recv().await {
            debug!(language, ?msg, "sending message to adapter");
            if let Err(e) = write_driver_message(&mut writer, &msg).await {
                match e {
                    CodecError::Io(ref io_err)
                        if io_err.kind() == std::io::ErrorKind::BrokenPipe
                            || io_err.kind() == std::io::ErrorKind::ConnectionReset =>
                    {
                        warn!(language, "adapter disconnected (write)");
                        break;
                    }
                    _ => {
                        error!(language, %e, "write error to adapter");
                        break;
                    }
                }
            }
        }
        debug!(language, "write loop ended");
    }

    /// Read loop: reads length-prefixed MessagePack frames from the socket
    /// and sends decoded messages into the incoming channel.
    async fn read_loop(
        mut reader: OwnedReadHalf,
        tx: mpsc::Sender<AdapterMessage>,
        language: &str,
    ) {
        loop {
            match read_adapter_message(&mut reader).await {
                Ok(msg) => {
                    debug!(language, ?msg, "received message from adapter");
                    if tx.send(msg).await.is_err() {
                        warn!(language, "incoming channel closed, stopping read loop");
                        break;
                    }
                }
                Err(CodecError::Closed) => {
                    warn!(language, "adapter disconnected (read)");
                    break;
                }
                Err(e) => {
                    error!(language, %e, "read error from adapter");
                    break;
                }
            }
        }
        debug!(language, "read loop ended");
    }
}
