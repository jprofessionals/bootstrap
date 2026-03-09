//! Connection/communication kfuns (driver service stubs): send_message, users,
//! query_ip_number, query_ip_name, connect, connect_datagram, datagram_challenge,
//! send_close, send_datagram, block_input.
//!
//! These kfuns route through driver services via MOP. All are stubs until
//! MOP integration is complete.

use crate::bytecode::LpcValue;
use super::{KfunContext, LpcError};

/// send_message(mixed msg) -> int
///
/// Send data to the current user's connection.
/// - If msg is string: send as text
/// - If msg is int: 0 = disable echo, 1 = enable echo
/// Returns number of bytes sent.
pub fn kf_send_message(
    _ctx: &mut KfunContext,
    _args: &[LpcValue],
) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "send_message: driver services not yet connected".into(),
    ))
}

/// users() -> object*
///
/// Returns array of all connected user objects.
pub fn kf_users(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "users: driver services not yet connected".into(),
    ))
}

/// query_ip_number(object user) -> string
///
/// Returns IP address of a user's connection.
pub fn kf_query_ip_number(
    _ctx: &mut KfunContext,
    _args: &[LpcValue],
) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "query_ip_number: driver services not yet connected".into(),
    ))
}

/// query_ip_name(object user) -> string
///
/// Returns hostname of a user's connection.
pub fn kf_query_ip_name(
    _ctx: &mut KfunContext,
    _args: &[LpcValue],
) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "query_ip_name: driver services not yet connected".into(),
    ))
}

/// connect(string host, int port) -> void
///
/// Initiate an outbound TCP connection to host:port.
pub fn kf_connect(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "connect: driver services not yet connected".into(),
    ))
}

/// connect_datagram(string host, int port) -> void
///
/// Initiate an outbound datagram (UDP) connection.
pub fn kf_connect_datagram(
    _ctx: &mut KfunContext,
    _args: &[LpcValue],
) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "connect_datagram: driver services not yet connected".into(),
    ))
}

/// datagram_challenge(string challenge) -> void
///
/// Set the datagram challenge string for connection authentication.
pub fn kf_datagram_challenge(
    _ctx: &mut KfunContext,
    _args: &[LpcValue],
) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "datagram_challenge: driver services not yet connected".into(),
    ))
}

/// send_close() -> void
///
/// Close the output side of a connection.
pub fn kf_send_close(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "send_close: driver services not yet connected".into(),
    ))
}

/// send_datagram(string data) -> int
///
/// Send a datagram message on the current connection.
pub fn kf_send_datagram(
    _ctx: &mut KfunContext,
    _args: &[LpcValue],
) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "send_datagram: driver services not yet connected".into(),
    ))
}

/// block_input(int flag) -> void
///
/// Block (1) or unblock (0) user input on the current connection.
pub fn kf_block_input(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "block_input: driver services not yet connected".into(),
    ))
}
