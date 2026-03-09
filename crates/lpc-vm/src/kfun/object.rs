//! Object management kfuns: this_object, previous_object, clone_object,
//! new_object, destruct_object, find_object, object_name, function_object,
//! compile_object, this_user, call_touch, call_other, previous_program.

use crate::bytecode::LpcValue;
use super::{KfunContext, LpcError, require_object, require_string};

/// this_object() -> object
///
/// Returns the current object (the one whose function is executing).
pub fn kf_this_object(ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    Ok(LpcValue::Object(ctx.this_object.clone()))
}

/// previous_object(varargs int depth) -> object
///
/// Returns the object that called the current function. With depth argument,
/// walks further back in the call stack.
///
/// TODO: Full implementation requires VM call stack access.
/// Currently returns previous_object from context (depth 0 only).
pub fn kf_previous_object(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    if !args.is_empty() {
        let depth = super::require_int(&args[0], 0)?;
        if depth < 0 {
            return Err(LpcError::ValueError("negative depth".into()));
        }
        if depth > 0 {
            // TODO: Walk call stack for depth > 0
            return Ok(LpcValue::Nil);
        }
    }
    match &ctx.previous_object {
        Some(obj_ref) => Ok(LpcValue::Object((*obj_ref).clone())),
        None => Ok(LpcValue::Nil),
    }
}

/// clone_object(object master) -> object
///
/// Create a clone of a master object. Clones share the master's program but
/// have their own variable state.
///
/// TODO: Full implementation requires VM object table.
/// Currently returns an error indicating driver services are needed.
pub fn kf_clone_object(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let master = require_object(&args[0], 0)?;
    if master.is_lightweight {
        return Err(LpcError::RuntimeError(
            "cannot clone lightweight object".into(),
        ));
    }
    // TODO: Requires VM object table integration
    Err(LpcError::RuntimeError(
        "clone_object: VM object table not yet connected".into(),
    ))
}

/// new_object(object master) -> object
///
/// Create a lightweight object. Lightweight objects are reference-counted and
/// automatically deallocated when no references remain.
///
/// TODO: Full implementation requires VM object table.
pub fn kf_new_object(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let _master = require_object(&args[0], 0)?;
    // TODO: Requires VM object table integration
    Err(LpcError::RuntimeError(
        "new_object: VM object table not yet connected".into(),
    ))
}

/// destruct_object(object obj) -> void
///
/// Destroy an object, removing it from the object table.
///
/// TODO: Full implementation requires VM object table.
pub fn kf_destruct_object(
    _ctx: &mut KfunContext,
    args: &[LpcValue],
) -> Result<LpcValue, LpcError> {
    let _obj = require_object(&args[0], 0)?;
    // TODO: Requires VM object table integration
    Err(LpcError::RuntimeError(
        "destruct_object: VM object table not yet connected".into(),
    ))
}

/// find_object(string path) -> object
///
/// Find a compiled object by its path. Returns nil if not found.
///
/// TODO: Full implementation requires VM object table.
pub fn kf_find_object(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let _path = require_string(&args[0], 0)?;
    // TODO: Requires VM object table integration
    // For now, return nil (object not found)
    Ok(LpcValue::Nil)
}

/// object_name(object obj) -> string
///
/// Returns the path name of an object. For clones, appends #N.
/// For lightweight objects, appends #-1.
pub fn kf_object_name(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let obj = require_object(&args[0], 0)?;
    if obj.is_lightweight {
        Ok(LpcValue::String(format!("{}#-1", obj.path)))
    } else {
        // For regular objects, the path itself is the name.
        // For clones, the id distinguishes them.
        Ok(LpcValue::String(obj.path.clone()))
    }
}

/// function_object(string func, object obj) -> string
///
/// Returns the path of the program that defines the named function in the
/// given object. Returns nil if the function doesn't exist.
///
/// TODO: Full implementation requires VM program table.
pub fn kf_function_object(
    _ctx: &mut KfunContext,
    args: &[LpcValue],
) -> Result<LpcValue, LpcError> {
    let _func = require_string(&args[0], 0)?;
    let obj = require_object(&args[1], 1)?;
    // Stub: return the object's own path as the function origin
    Ok(LpcValue::String(obj.path.clone()))
}

/// compile_object(string path, string... sources) -> object
///
/// Compile (or recompile) an LPC source file.
///
/// TODO: Full implementation requires driver services (file I/O, compiler).
pub fn kf_compile_object(
    _ctx: &mut KfunContext,
    args: &[LpcValue],
) -> Result<LpcValue, LpcError> {
    let _path = require_string(&args[0], 0)?;
    // TODO: Requires driver services integration (MOP)
    Err(LpcError::RuntimeError(
        "compile_object: driver services not yet connected".into(),
    ))
}

/// this_user() -> object
///
/// Returns the current user object (the one associated with the current
/// execution context). Returns nil if not in a user context.
///
/// TODO: Full implementation requires VM user session tracking.
pub fn kf_this_user(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    // TODO: Requires VM user session tracking
    Ok(LpcValue::Nil)
}

/// call_touch(object obj) -> int
///
/// Register a callback on the next function call to the given object.
/// When the object is next "touched" (has a function called on it),
/// the driver calls `touch(obj)` in the current object first.
pub fn kf_call_touch(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "call_touch: not yet implemented".into(),
    ))
}

/// call_other(object obj, string func, args...) -> mixed
///
/// Call a function in another object. Equivalent to `obj->func(args...)`.
pub fn kf_call_other(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "call_other: use obj->func() syntax or driver services not connected".into(),
    ))
}

/// previous_program(varargs int depth) -> string
///
/// Returns the program name from the call stack at the given depth.
/// Without arguments, returns the program that called the current function.
pub fn kf_previous_program(
    _ctx: &mut KfunContext,
    _args: &[LpcValue],
) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "previous_program: call stack inspection not yet connected".into(),
    ))
}
