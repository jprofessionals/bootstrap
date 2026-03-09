//! Type inspection kfuns: typeof, instanceof.

use crate::bytecode::LpcValue;
use super::{KfunContext, LpcError, require_object, require_string};
use super::{T_NIL, T_INT, T_FLOAT, T_STRING, T_OBJECT, T_ARRAY, T_MAPPING, T_LWOBJECT};

/// typeof(mixed value) -> int
///
/// Returns the type constant for the given value.
pub fn kf_typeof(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let type_id = match &args[0] {
        LpcValue::Nil => T_NIL,
        LpcValue::Int(_) => T_INT,
        LpcValue::Float(_) => T_FLOAT,
        LpcValue::String(_) => T_STRING,
        LpcValue::Object(r) if r.is_lightweight => T_LWOBJECT,
        LpcValue::Object(_) => T_OBJECT,
        LpcValue::Array(_) => T_ARRAY,
        LpcValue::Mapping(_) => T_MAPPING,
    };
    Ok(LpcValue::Int(type_id))
}

/// instanceof(object obj, string type_name) -> int
///
/// Checks if an object's program inherits from the named type.
/// Returns 1 if it does, 0 otherwise.
///
/// TODO: Full implementation requires VM object table to resolve inheritance.
/// Currently checks if the object path contains the type name.
pub fn kf_instanceof(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let obj = require_object(&args[0], 0)?;
    let type_name = require_string(&args[1], 1)?;

    // Stub: check if the object path matches the type name.
    // Full implementation needs the VM's program/inheritance table.
    let matches = obj.path == type_name || obj.path.ends_with(&format!("/{}", type_name));
    Ok(LpcValue::Int(if matches { 1 } else { 0 }))
}
