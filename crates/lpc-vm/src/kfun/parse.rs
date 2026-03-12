//! Parsing kfuns: parse_string.
//!
//! DGD provides a built-in LR parser for grammar-based string parsing.
//! This is a stub until the parser is implemented.

use super::{KfunContext, LpcError};
use crate::bytecode::LpcValue;

/// parse_string(string grammar, string input, varargs args...) -> mixed*
///
/// Parse a string against an LR grammar. Returns an array of matched
/// tokens/results, or nil if parsing fails.
pub fn kf_parse_string(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "parse_string: LR parser not yet implemented".into(),
    ))
}
