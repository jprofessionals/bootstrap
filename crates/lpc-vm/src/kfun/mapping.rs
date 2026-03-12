//! Mapping operation kfuns: map_indices, map_values, map_sizeof, mkmapping.

use super::{require_array, require_mapping, KfunContext, LpcError};
use crate::bytecode::LpcValue;

/// map_indices(mapping m) -> mixed*
///
/// Returns array of all keys in the mapping.
pub fn kf_map_indices(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let m = require_mapping(&args[0], 0)?;
    *ctx.tick_counter = ctx.tick_counter.saturating_sub(m.len() as u64);
    let keys: Vec<LpcValue> = m.iter().map(|(k, _)| k.clone()).collect();
    Ok(LpcValue::Array(keys))
}

/// map_values(mapping m) -> mixed*
///
/// Returns array of all values in the mapping.
pub fn kf_map_values(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let m = require_mapping(&args[0], 0)?;
    *ctx.tick_counter = ctx.tick_counter.saturating_sub(m.len() as u64);
    let vals: Vec<LpcValue> = m.iter().map(|(_, v)| v.clone()).collect();
    Ok(LpcValue::Array(vals))
}

/// map_sizeof(mapping m) -> int
///
/// Returns number of key-value pairs. Same as sizeof() for mappings.
pub fn kf_map_sizeof(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let m = require_mapping(&args[0], 0)?;
    Ok(LpcValue::Int(m.len() as i64))
}

/// mkmapping(mixed* keys, mixed* values) -> mapping
///
/// Create a mapping from parallel arrays of keys and values.
/// Arrays must be the same length.
pub fn kf_mkmapping(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let keys = require_array(&args[0], 0)?;
    let vals = require_array(&args[1], 1)?;
    if keys.len() != vals.len() {
        return Err(LpcError::ValueError(
            "key and value arrays must be same length".into(),
        ));
    }
    *ctx.tick_counter = ctx.tick_counter.saturating_sub(keys.len() as u64);
    let pairs: Vec<(LpcValue, LpcValue)> = keys
        .iter()
        .zip(vals.iter())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    Ok(LpcValue::Mapping(pairs))
}
