//! Array operation kfuns: allocate, allocate_int, allocate_float, sizeof, sort_array.

use crate::bytecode::LpcValue;
use super::{KfunContext, LpcError, require_int, require_string, require_array};

/// allocate(int size) -> mixed*
///
/// Create array of given size, all elements initialized to nil.
pub fn kf_allocate(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let size = require_int(&args[0], 0)?;
    if size < 0 {
        return Err(LpcError::ValueError("negative array size".into()));
    }
    *ctx.tick_counter = ctx.tick_counter.saturating_sub(size as u64);
    Ok(LpcValue::Array(vec![LpcValue::Nil; size as usize]))
}

/// allocate_int(int size) -> int*
///
/// Create array of given size, all elements initialized to 0.
pub fn kf_allocate_int(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let size = require_int(&args[0], 0)?;
    if size < 0 {
        return Err(LpcError::ValueError("negative array size".into()));
    }
    *ctx.tick_counter = ctx.tick_counter.saturating_sub(size as u64);
    Ok(LpcValue::Array(vec![LpcValue::Int(0); size as usize]))
}

/// allocate_float(int size) -> float*
///
/// Create array of given size, all elements initialized to 0.0.
pub fn kf_allocate_float(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let size = require_int(&args[0], 0)?;
    if size < 0 {
        return Err(LpcError::ValueError("negative array size".into()));
    }
    *ctx.tick_counter = ctx.tick_counter.saturating_sub(size as u64);
    Ok(LpcValue::Array(vec![LpcValue::Float(0.0); size as usize]))
}

/// sizeof(mixed value) -> int
///
/// Returns size of array, mapping, or string. For other types, returns 0.
pub fn kf_sizeof(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let size = match &args[0] {
        LpcValue::Array(a) => a.len() as i64,
        LpcValue::Mapping(m) => m.len() as i64,
        LpcValue::String(s) => s.len() as i64,
        _ => 0,
    };
    Ok(LpcValue::Int(size))
}

/// sort_array(mixed* arr, string compare_func) -> mixed*
///
/// Sort array using a comparison function. The compare function name is stored
/// but actual callback invocation requires the full VM. For now, this performs
/// a default sort: ints/floats numerically, strings lexicographically.
///
/// TODO: Full implementation requires VM call_function to invoke the named
/// comparison function on the current object.
pub fn kf_sort_array(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let arr = require_array(&args[0], 0)?;
    let _func_name = require_string(&args[1], 1)?;

    let mut sorted = arr.to_vec();

    // Default sort: compare by type, then by value within type.
    // Full VM integration will call the named comparison function instead.
    sorted.sort_by(|a, b| {
        match (a, b) {
            (LpcValue::Int(x), LpcValue::Int(y)) => x.cmp(y),
            (LpcValue::Float(x), LpcValue::Float(y)) => {
                x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)
            }
            (LpcValue::String(x), LpcValue::String(y)) => x.cmp(y),
            // Different types: sort by type discriminant
            _ => {
                let type_ord =
                    |v: &LpcValue| -> u8 {
                        match v {
                            LpcValue::Nil => 0,
                            LpcValue::Int(_) => 1,
                            LpcValue::Float(_) => 2,
                            LpcValue::String(_) => 3,
                            LpcValue::Array(_) => 4,
                            LpcValue::Mapping(_) => 5,
                            LpcValue::Object(_) => 6,
                        }
                    };
                type_ord(a).cmp(&type_ord(b))
            }
        }
    });

    Ok(LpcValue::Array(sorted))
}
