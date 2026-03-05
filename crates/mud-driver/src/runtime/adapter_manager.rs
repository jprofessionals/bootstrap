use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use mud_mop::codec::read_adapter_message;
use mud_mop::message::{AdapterMessage, DriverMessage};
use tokio::net::UnixListener;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use super::adapter_connection::AdapterConnection;
use crate::config::Config;

/// Manages all language adapter processes and their socket connections.
///
/// The driver communicates with each adapter over a Unix domain socket. This
/// manager handles spawning adapter child processes, accepting their
/// connections, and routing messages between the driver core and the adapters.
pub struct AdapterManager {
    socket_path: PathBuf,
    adapters: HashMap<String, AdapterConnection>,
    processes: Vec<Child>,
    incoming_tx: mpsc::Sender<AdapterMessage>,
    incoming_rx: mpsc::Receiver<AdapterMessage>,
}

impl AdapterManager {
    /// Create a new adapter manager that will listen on the given socket path.
    pub fn new(socket_path: PathBuf) -> Self {
        let (incoming_tx, incoming_rx) = mpsc::channel(1024);
        Self {
            socket_path,
            adapters: HashMap::new(),
            processes: Vec::new(),
            incoming_tx,
            incoming_rx,
        }
    }

    /// Bind the Unix listener and spawn configured adapter processes.
    ///
    /// Returns the listener so the caller can drive `accept_connection` in its
    /// own event loop.
    pub async fn start(&mut self, config: &Config) -> Result<UnixListener> {
        // Remove stale socket file if it exists.
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path).with_context(|| {
                format!(
                    "removing stale socket at {}",
                    self.socket_path.display()
                )
            })?;
        }

        // Create parent directories.
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("creating socket directory {}", parent.display())
            })?;
        }

        let listener = UnixListener::bind(&self.socket_path).with_context(|| {
            format!("binding Unix socket at {}", self.socket_path.display())
        })?;
        info!(path = %self.socket_path.display(), "listening for adapter connections");

        // Spawn configured adapters.
        if let Some(ref ruby) = config.adapters.ruby {
            if ruby.enabled {
                let socket_str = self.socket_path.to_string_lossy().to_string();
                let world_path = config.world.resolved_path();
                self.spawn_adapter(&ruby.command, &ruby.adapter_path, &socket_str, &world_path)?;
                info!("spawned Ruby adapter process");
            }
        }

        Ok(listener)
    }

    /// Accept an incoming adapter connection on the listener.
    ///
    /// Reads the initial handshake message, sets up bidirectional
    /// communication, and returns the language identifier string.
    pub async fn accept_connection(&mut self, listener: &UnixListener) -> Result<String> {
        let (stream, _addr) = listener.accept().await.context("accepting adapter connection")?;
        let (mut reader, writer) = stream.into_split();

        // The first message must be a Handshake.
        let first_msg = read_adapter_message(&mut reader)
            .await
            .context("reading handshake from adapter")?;

        let (adapter_name, language, version) = match first_msg {
            AdapterMessage::Handshake {
                adapter_name,
                language,
                version,
            } => (adapter_name, language, version),
            other => {
                bail!(
                    "expected Handshake as first message, got {:?}",
                    std::mem::discriminant(&other)
                );
            }
        };

        info!(
            adapter_name,
            language,
            version,
            "adapter connected"
        );

        let (conn, mut adapter_rx) = AdapterConnection::spawn(reader, writer, adapter_name, language.clone());

        // Forward this adapter's messages into the merged incoming channel.
        let merged_tx = self.incoming_tx.clone();
        let lang_fwd = language.clone();
        tokio::spawn(async move {
            while let Some(msg) = adapter_rx.recv().await {
                if merged_tx.send(msg).await.is_err() {
                    warn!(language = %lang_fwd, "merged incoming channel closed");
                    break;
                }
            }
        });

        self.adapters.insert(language.clone(), conn);
        Ok(language)
    }

    /// Send a message to a specific adapter identified by language.
    pub async fn send_to(&self, language: &str, msg: DriverMessage) -> Result<()> {
        let conn = self
            .adapters
            .get(language)
            .ok_or_else(|| anyhow::anyhow!("no adapter connected for language: {language}"))?;
        conn.send(msg).await
    }

    /// Receive the next message from any connected adapter.
    pub async fn recv(&mut self) -> Option<AdapterMessage> {
        self.incoming_rx.recv().await
    }

    /// Check whether an adapter for the given language is connected.
    #[allow(dead_code)]
    pub fn has_adapter(&self, language: &str) -> bool {
        self.adapters.contains_key(language)
    }

    /// Return the socket path this manager is bound to.
    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    /// Spawn a language adapter as a child process.
    ///
    /// The adapter binary is expected at `adapter_path` and will be invoked
    /// with the given `command` (e.g. `ruby`) plus `--socket <socket_path>`.
    fn spawn_adapter(
        &mut self,
        command: &str,
        adapter_path: &str,
        socket_path: &str,
        world_path: &std::path::Path,
    ) -> Result<()> {
        let child = Command::new(command)
            .arg(adapter_path)
            .arg("--socket")
            .arg(socket_path)
            .env("MUD_WORLD_PATH", world_path)
            .kill_on_drop(true)
            .spawn()
            .with_context(|| {
                format!("spawning adapter: {command} {adapter_path} --socket {socket_path}")
            })?;

        self.processes.push(child);
        Ok(())
    }

    /// Shut down all adapters and clean up resources.
    pub fn shutdown(&mut self) {
        self.adapters.clear();

        for mut child in self.processes.drain(..) {
            if let Err(e) = child.start_kill() {
                error!(%e, "failed to kill adapter process");
            }
        }

        if self.socket_path.exists() {
            if let Err(e) = std::fs::remove_file(&self.socket_path) {
                warn!(path = %self.socket_path.display(), %e, "failed to remove socket file");
            }
        }

        info!("adapter manager shut down");
    }
}
