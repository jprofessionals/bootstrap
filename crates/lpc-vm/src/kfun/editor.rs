//! Editor kfuns (DGD-specific stubs): editor, query_editor.
//!
//! DGD provides a built-in ed-style line editor. These kfuns interface with it.
//! Our driver does not implement this feature, so these are permanent stubs.

use super::{KfunContext, LpcError};
use crate::bytecode::LpcValue;

/// editor(string command) -> string
///
/// Execute an ed-style editor command. This is a DGD-specific feature
/// that provides an in-MUD line editor.
pub fn kf_editor(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "editor: not implemented (DGD-specific feature)".into(),
    ))
}

/// query_editor(object obj) -> string
///
/// Query the editor mode of an object. Returns nil if no editor is active,
/// or a string indicating the current editor mode.
pub fn kf_query_editor(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    // No editor is ever active in our driver
    Ok(LpcValue::Nil)
}
