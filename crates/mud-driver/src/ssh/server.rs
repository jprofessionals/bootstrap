use std::sync::Arc;

use russh::keys::ssh_key::rand_core::OsRng;
use russh::keys::{Algorithm, PrivateKey};
use russh::server::Server as _;
use tokio::sync::mpsc;

use super::handler::{SshCommand, SshHandler};
use crate::config::SshConfig;
use crate::persistence::player_store::PlayerStore;

// ---------------------------------------------------------------------------
// MudSshServer — the `russh::server::Server` factory
// ---------------------------------------------------------------------------

/// Factory that creates a new [`SshHandler`] for every incoming connection.
struct MudSshServer {
    command_tx: mpsc::Sender<SshCommand>,
    player_store: Option<Arc<PlayerStore>>,
}

impl russh::server::Server for MudSshServer {
    type Handler = SshHandler;

    fn new_client(&mut self, peer_addr: Option<std::net::SocketAddr>) -> SshHandler {
        tracing::info!(?peer_addr, "New SSH connection");
        SshHandler::new(self.command_tx.clone(), self.player_store.clone())
    }

    fn handle_session_error(&mut self, error: <SshHandler as russh::server::Handler>::Error) {
        tracing::warn!(?error, "SSH session error");
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Start the SSH server.
///
/// This binds to `config.host:config.port`, generates (or loads) a host key,
/// and runs the russh accept loop.  Each accepted connection gets its own
/// [`SshHandler`] which communicates with the server orchestrator via
/// `command_tx`.
///
/// The returned future resolves when the server shuts down.
pub async fn start_ssh_server(
    config: &SshConfig,
    command_tx: mpsc::Sender<SshCommand>,
    player_store: Option<Arc<PlayerStore>>,
) -> anyhow::Result<()> {
    // -- Host key -----------------------------------------------------------
    let host_key = match &config.host_key {
        Some(path) => {
            tracing::info!(path, "Loading SSH host key from file");
            russh::keys::decode_secret_key(&std::fs::read_to_string(path)?, None)
                .map_err(|e| anyhow::anyhow!("Failed to decode host key: {e}"))?
        }
        None => {
            tracing::info!("Generating ephemeral Ed25519 host key");
            PrivateKey::random(&mut OsRng, Algorithm::Ed25519)
                .map_err(|e| anyhow::anyhow!("Failed to generate host key: {e}"))?
        }
    };

    // -- russh server config ------------------------------------------------
    let server_config = russh::server::Config {
        inactivity_timeout: Some(std::time::Duration::from_secs(3600)),
        auth_rejection_time: std::time::Duration::from_secs(3),
        auth_rejection_time_initial: Some(std::time::Duration::from_secs(0)),
        keys: vec![host_key],
        ..Default::default()
    };
    let server_config = Arc::new(server_config);

    // -- Bind & run ---------------------------------------------------------
    let addr: std::net::SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    tracing::info!(%addr, "SSH server listening");

    let tcp_socket = tokio::net::TcpSocket::new_v4()?;
    tcp_socket.set_reuseaddr(true)?;
    tcp_socket.bind(addr)?;
    let socket = tcp_socket.listen(128)?;

    let mut server = MudSshServer {
        command_tx,
        player_store,
    };
    server.run_on_socket(server_config, &socket).await?;

    Ok(())
}
