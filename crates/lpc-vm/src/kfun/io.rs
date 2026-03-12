//! File I/O kfuns (driver service stubs): read_file, write_file, remove_file,
//! rename_file, get_dir, make_dir, remove_dir.
//!
//! These kfuns route through driver services via MOP. All are stubs until
//! MOP integration is complete.

use super::{KfunContext, LpcError};
use crate::bytecode::LpcValue;

/// read_file(string path, varargs int start, int lines) -> string
///
/// Read file contents. Optional start line and line count.
pub fn kf_read_file(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "read_file: driver services not yet connected".into(),
    ))
}

/// write_file(string path, string content, varargs int offset) -> int
///
/// Write content to file. Returns 1 on success.
pub fn kf_write_file(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "write_file: driver services not yet connected".into(),
    ))
}

/// remove_file(string path) -> int
///
/// Delete a file. Returns 1 on success, 0 on failure.
pub fn kf_remove_file(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "remove_file: driver services not yet connected".into(),
    ))
}

/// rename_file(string from, string to) -> int
///
/// Rename/move a file. Returns 1 on success, 0 on failure.
pub fn kf_rename_file(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "rename_file: driver services not yet connected".into(),
    ))
}

/// get_dir(string pattern) -> mixed**
///
/// List directory contents matching pattern. Returns ({names, sizes, timestamps}).
pub fn kf_get_dir(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "get_dir: driver services not yet connected".into(),
    ))
}

/// make_dir(string path) -> int
///
/// Create a directory. Returns 1 on success, 0 on failure.
pub fn kf_make_dir(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "make_dir: driver services not yet connected".into(),
    ))
}

/// remove_dir(string path) -> int
///
/// Remove an empty directory. Returns 1 on success, 0 on failure.
pub fn kf_remove_dir(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "remove_dir: driver services not yet connected".into(),
    ))
}
