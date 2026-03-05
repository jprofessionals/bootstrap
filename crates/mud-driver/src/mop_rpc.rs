//! MOP RPC client — allows web handlers to send a [`DriverMessage`] to the
//! adapter and await the response.
//!
//! The [`MopRpcClient`] is cheaply clonable and is stored in axum shared state
//! (e.g. `AppState` or `EditorState`).  It communicates with the [`Server`]
//! event loop via an `mpsc` channel.
//!
//! Flow:
//! 1. Web handler calls `mop_rpc.call(driver_message)`.
//! 2. `MopRpcClient` packages the message together with a oneshot response
//!    channel and sends it to the server event loop.
//! 3. The server event loop assigns a request ID, sends the message to the
//!    adapter, and stores the oneshot sender keyed by that request ID.
//! 4. When the adapter replies (`CallResult` / `CallError`), the server
//!    looks up the oneshot sender and delivers the result.
//! 5. The web handler receives the result on the oneshot receiver.

use mud_mop::message::{DriverMessage, Value};
use tokio::sync::{mpsc, oneshot};

/// A request submitted by a web handler destined for the adapter.
pub struct MopRequest {
    /// The driver message to send to the adapter.
    pub message: DriverMessage,
    /// Channel on which the server will deliver the adapter's response.
    pub response_tx: oneshot::Sender<Result<Value, String>>,
}

/// Cheaply-clonable handle that web handlers use to call into the adapter
/// via the server event loop.
#[derive(Clone)]
pub struct MopRpcClient {
    tx: mpsc::Sender<MopRequest>,
}

impl MopRpcClient {
    /// Create a new client from the sending half of the MOP RPC channel.
    pub fn new(tx: mpsc::Sender<MopRequest>) -> Self {
        Self { tx }
    }

    /// Send a [`DriverMessage`] to the adapter and wait for the response.
    ///
    /// The caller should construct the `DriverMessage` variant with
    /// `request_id: 0` — the server event loop will replace it with a
    /// unique value before forwarding to the adapter.
    pub async fn call(&self, message: DriverMessage) -> Result<Value, String> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(MopRequest {
                message,
                response_tx,
            })
            .await
            .map_err(|_| "MOP RPC channel closed".to_string())?;
        response_rx
            .await
            .map_err(|_| "MOP RPC response channel closed".to_string())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn call_returns_response() {
        let (tx, mut rx) = mpsc::channel::<MopRequest>(4);
        let client = MopRpcClient::new(tx);

        // Spawn a fake "server" that always responds with success.
        tokio::spawn(async move {
            if let Some(req) = rx.recv().await {
                let _ = req
                    .response_tx
                    .send(Ok(Value::String("allowed".into())));
            }
        });

        let result = client
            .call(DriverMessage::CheckBuilderAccess {
                request_id: 0,
                user: "alice".into(),
                namespace: "ns".into(),
                area: "dungeon".into(),
                action: "write".into(),
            })
            .await;

        assert_eq!(result, Ok(Value::String("allowed".into())));
    }

    #[tokio::test]
    async fn call_returns_error() {
        let (tx, mut rx) = mpsc::channel::<MopRequest>(4);
        let client = MopRpcClient::new(tx);

        tokio::spawn(async move {
            if let Some(req) = rx.recv().await {
                let _ = req.response_tx.send(Err("denied".into()));
            }
        });

        let result = client
            .call(DriverMessage::CheckBuilderAccess {
                request_id: 0,
                user: "bob".into(),
                namespace: "ns".into(),
                area: "dungeon".into(),
                action: "write".into(),
            })
            .await;

        assert_eq!(result, Err("denied".into()));
    }

    #[tokio::test]
    async fn call_fails_when_channel_closed() {
        let (tx, rx) = mpsc::channel::<MopRequest>(4);
        let client = MopRpcClient::new(tx);
        drop(rx);

        let result = client
            .call(DriverMessage::CheckBuilderAccess {
                request_id: 0,
                user: "alice".into(),
                namespace: "ns".into(),
                area: "dungeon".into(),
                action: "write".into(),
            })
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("closed"));
    }
}
