//! Miscellaneous kfuns: error, call_trace, status, dump_state, shutdown, swapout.

use crate::bytecode::LpcValue;
use super::{KfunContext, LpcError, require_string};

/// error(string msg) -> void
///
/// Throw a runtime error. Equivalent to `throw` in other languages.
pub fn kf_error(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let msg = require_string(&args[0], 0)?;
    Err(LpcError::RuntimeError(msg.to_string()))
}

/// call_trace() -> mixed**
///
/// Returns the current call stack as an array of arrays. Each entry contains:
/// ({object, function, file, line, is_external}).
///
/// TODO: Full implementation requires VM call stack access.
/// Currently returns an empty array.
pub fn kf_call_trace(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    // TODO: Requires VM call stack access
    Ok(LpcValue::Array(Vec::new()))
}

/// status(varargs object obj) -> mixed*
///
/// Returns resource usage information. Without argument, returns system-wide
/// status. With object argument, returns that object's status.
///
/// TODO: Full implementation requires VM resource tracking.
/// Currently returns a stub array with placeholder values.
pub fn kf_status(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    // Return stub status array with placeholder values
    Ok(LpcValue::Array(vec![
        LpcValue::Int(0),   // uptime
        LpcValue::Int(0),   // swap_size
        LpcValue::Int(0),   // swap_used
        LpcValue::Int(512), // sector_size
        LpcValue::Int(0),   // free_sectors
        LpcValue::Int(0),   // total objects
        LpcValue::Int(0),   // pending call_outs
    ]))
}

/// dump_state(varargs int incremental) -> void
///
/// Dump persistent state to a snapshot file.
///
/// TODO: Full implementation requires driver services.
pub fn kf_dump_state(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    // TODO: Requires driver services integration
    Err(LpcError::RuntimeError(
        "dump_state: driver services not yet connected".into(),
    ))
}

/// shutdown(varargs int hotboot) -> void
///
/// Shut down the driver. If hotboot is 1, perform a hotboot.
///
/// TODO: Full implementation requires driver services.
pub fn kf_shutdown(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    // TODO: Requires driver services integration
    Err(LpcError::RuntimeError(
        "shutdown: driver services not yet connected".into(),
    ))
}

/// swapout() -> void
///
/// Swap out all objects to disk. No-op in our VM since we don't use
/// disk-based object swapping.
pub fn kf_swapout(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    // No-op: our VM doesn't swap objects to disk
    Ok(LpcValue::Nil)
}
