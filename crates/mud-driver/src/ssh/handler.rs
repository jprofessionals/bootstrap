use std::collections::HashMap;
use std::sync::Arc;

use russh::server::{Auth, Msg, Session};
use russh::{Channel, ChannelId, CryptoVec};
use tokio::sync::{mpsc, oneshot};

use crate::persistence::player_store::{AuthResult, PlayerStore};

// ---------------------------------------------------------------------------
// SshCommand — messages sent from an SSH handler to the server orchestrator
// ---------------------------------------------------------------------------

/// Commands emitted by the per-connection SSH handler toward the
/// central server loop.
#[derive(Debug)]
pub enum SshCommand {
    /// A client has opened a shell and is ready for interaction.
    NewSession {
        username: String,
        /// The server can push output bytes into this sender, and the SSH
        /// handler will forward them to the client.
        output_tx: mpsc::Sender<Vec<u8>>,
        /// The server sends back the assigned session ID through this channel.
        session_id_tx: oneshot::Sender<u64>,
    },
    /// The client sent a complete line of input.
    Input { session_id: u64, line: String },
    /// The client disconnected.
    Disconnect { session_id: u64 },
}

// ---------------------------------------------------------------------------
// SshHandler — per-connection russh handler
// ---------------------------------------------------------------------------

/// Per-connection SSH handler.
///
/// Implements [`russh::server::Handler`] to authenticate users,
/// open shell sessions, perform line-buffered I/O with local echo,
/// and relay completed lines to the server orchestrator via
/// [`SshCommand`].
pub struct SshHandler {
    /// Set after successful authentication.
    username: Option<String>,
    /// Channel used to send commands to the server orchestrator.
    command_tx: mpsc::Sender<SshCommand>,
    /// Per-channel line buffer — accumulates characters until CR/LF.
    line_buffer: HashMap<ChannelId, String>,
    /// Assigned by the server once `NewSession` is received.
    pub session_id: Option<u64>,
    /// Optional player store for password verification.
    player_store: Option<Arc<PlayerStore>>,
}

impl SshHandler {
    pub fn new(
        command_tx: mpsc::Sender<SshCommand>,
        player_store: Option<Arc<PlayerStore>>,
    ) -> Self {
        Self {
            username: None,
            command_tx,
            line_buffer: HashMap::new(),
            session_id: None,
            player_store,
        }
    }
}

impl russh::server::Handler for SshHandler {
    type Error = russh::Error;

    /// Verify password against PlayerStore if available, otherwise accept
    /// any non-empty password (dev fallback).
    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        if password.is_empty() {
            return Ok(Auth::reject());
        }
        if let Some(ps) = &self.player_store {
            match ps.authenticate(user, password).await {
                Ok(AuthResult::Success(_)) => {
                    self.username = Some(user.to_string());
                    Ok(Auth::Accept)
                }
                _ => Ok(Auth::reject()),
            }
        } else {
            // No player store — accept any non-empty password (dev fallback)
            self.username = Some(user.to_string());
            Ok(Auth::Accept)
        }
    }

    /// Accept session channel opens.
    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    /// Handle PTY requests — accept them so that clients like OpenSSH
    /// can proceed to request a shell.
    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _col_width: u32,
        _row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        Ok(())
    }

    /// When the client requests a shell, create an output channel pair
    /// and notify the server orchestrator via [`SshCommand::NewSession`].
    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;

        let (output_tx, mut output_rx) = mpsc::channel::<Vec<u8>>(64);
        let (session_id_tx, session_id_rx) = oneshot::channel::<u64>();

        // Send NewSession notification to the server orchestrator.
        let username = self.username.clone().unwrap_or_else(|| "anonymous".into());
        let _ = self.command_tx.send(SshCommand::NewSession {
            username,
            output_tx,
            session_id_tx,
        }).await;

        // Wait for the server to assign us a session ID.
        if let Ok(id) = session_id_rx.await {
            self.session_id = Some(id);
        }

        // Spawn a task that forwards output bytes from the server
        // orchestrator to the SSH channel.
        let handle = session.handle();
        tokio::spawn(async move {
            while let Some(data) = output_rx.recv().await {
                let cv = CryptoVec::from(data);
                if handle.data(channel, cv).await.is_err() {
                    break;
                }
            }
        });

        // Initialize line buffer for this channel.
        self.line_buffer.entry(channel).or_default();

        // No welcome message here — the adapter sends its own welcome
        // via SessionOutput after receiving SessionStart.

        Ok(())
    }

    /// Handle incoming data: line-buffer with local echo.
    ///
    /// - Printable characters: echo back and append to buffer
    /// - Backspace (`\x7f`): erase last char if any, echo BS-SPACE-BS
    /// - CR (`\r`) or LF (`\n`): echo CRLF, send completed line as
    ///   [`SshCommand::Input`], reset buffer, print new prompt
    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let buf = self.line_buffer.entry(channel).or_default();

        for &byte in data {
            match byte {
                // Enter — send the completed line
                b'\r' | b'\n' => {
                    // Echo CR LF
                    session.data(channel, CryptoVec::from("\r\n"))?;

                    let line = std::mem::take(buf);
                    if let Some(session_id) = self.session_id {
                        let _ = self.command_tx.send(SshCommand::Input {
                            session_id,
                            line,
                        }).await;
                    }

                    // No prompt here — the adapter sends the prompt as
                    // part of its SessionOutput response.
                }

                // Backspace — erase last character
                0x7f | 0x08 => {
                    if !buf.is_empty() {
                        buf.pop();
                        // BS, overwrite with space, BS again
                        session.data(channel, CryptoVec::from("\x08 \x08"))?;
                    }
                }

                // Ctrl-C — disconnect
                0x03 => {
                    if let Some(session_id) = self.session_id {
                        let _ = self.command_tx.send(SshCommand::Disconnect {
                            session_id,
                        }).await;
                    }
                    return Err(russh::Error::Disconnect);
                }

                // Printable character — echo and buffer
                c if (0x20..0x7f).contains(&c) => {
                    buf.push(c as char);
                    session.data(channel, CryptoVec::from(std::slice::from_ref(&byte)))?;
                }

                // Ignore everything else
                _ => {}
            }
        }

        Ok(())
    }

    /// Notify orchestrator when a channel is closed.
    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.line_buffer.remove(&channel);
        if let Some(session_id) = self.session_id {
            let _ = self.command_tx.send(SshCommand::Disconnect {
                session_id,
            }).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_handler_new_has_no_username() {
        let (tx, _rx) = mpsc::channel(1);
        let handler = SshHandler::new(tx, None);
        assert!(handler.username.is_none());
        assert!(handler.session_id.is_none());
        assert!(handler.line_buffer.is_empty());
    }

    #[test]
    fn ssh_command_new_session_debug() {
        let (output_tx, _output_rx) = mpsc::channel(1);
        let (session_id_tx, _session_id_rx) = oneshot::channel();
        let cmd = SshCommand::NewSession {
            username: "alice".into(),
            output_tx,
            session_id_tx,
        };
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("NewSession"));
        assert!(debug.contains("alice"));
    }

    #[test]
    fn ssh_command_input_debug() {
        let cmd = SshCommand::Input {
            session_id: 42,
            line: "look".into(),
        };
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("Input"));
        assert!(debug.contains("42"));
        assert!(debug.contains("look"));
    }

    #[test]
    fn ssh_command_disconnect_debug() {
        let cmd = SshCommand::Disconnect { session_id: 7 };
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("Disconnect"));
        assert!(debug.contains("7"));
    }
}
