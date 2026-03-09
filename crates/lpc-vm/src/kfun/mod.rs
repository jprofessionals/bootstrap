//! DGD kernel function (kfun) registry and implementations.
//!
//! All 117+ DGD kernel functions organized by category.

pub mod type_ops;
pub mod string;
pub mod array;
pub mod mapping;
pub mod math;
pub mod asn;
pub mod object;
pub mod timing;
pub mod io;
pub mod connection;
pub mod serialize;
pub mod crypto;
pub mod misc;
pub mod editor;
pub mod parse;

use std::collections::HashMap;

use crate::bytecode::{LpcValue, ObjectRef};

/// Type constants used by `typeof()` and type-checking kfuns.
pub const T_NIL: i64 = 0;
pub const T_INT: i64 = 1;
pub const T_FLOAT: i64 = 2;
pub const T_STRING: i64 = 3;
pub const T_OBJECT: i64 = 4;
pub const T_ARRAY: i64 = 5;
pub const T_MAPPING: i64 = 6;
pub const T_LWOBJECT: i64 = 7;

/// Error type for kfun execution.
#[derive(Debug, thiserror::Error)]
pub enum LpcError {
    #[error("type error: expected {expected}, got {got} at argument {arg_pos}")]
    TypeError {
        expected: &'static str,
        got: String,
        arg_pos: usize,
    },
    #[error("value error: {0}")]
    ValueError(String),
    #[error("runtime error: {0}")]
    RuntimeError(String),
    #[error("atomic violation: {0}")]
    AtomicViolation(String),
}

/// Context provided to every kfun call.
pub struct KfunContext<'a> {
    pub this_object: &'a ObjectRef,
    pub previous_object: Option<&'a ObjectRef>,
    pub tick_counter: &'a mut u64,
    // Driver services will be added later when MOP integration happens
}

/// Signature for all kfun implementations.
pub type KfunFn = fn(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError>;

/// A registered kfun entry.
struct KfunEntry {
    name: String,
    func: KfunFn,
    min_args: u8,
    max_args: u8,
}

/// Registry mapping kfun names to implementations.
pub struct KfunRegistry {
    by_name: HashMap<String, u16>,
    entries: Vec<KfunEntry>,
}

impl KfunRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        KfunRegistry {
            by_name: HashMap::new(),
            entries: Vec::new(),
        }
    }

    /// Register a kfun and return its numeric ID.
    pub fn register(&mut self, name: &str, func: KfunFn, min_args: u8, max_args: u8) -> u16 {
        let id = self.entries.len() as u16;
        self.by_name.insert(name.to_string(), id);
        self.entries.push(KfunEntry {
            name: name.to_string(),
            func,
            min_args,
            max_args,
        });
        id
    }

    /// Look up a kfun ID by name.
    pub fn lookup(&self, name: &str) -> Option<u16> {
        self.by_name.get(name).copied()
    }

    /// Call a kfun by ID, validating argument count.
    pub fn call(
        &self,
        id: u16,
        ctx: &mut KfunContext,
        args: &[LpcValue],
    ) -> Result<LpcValue, LpcError> {
        let entry = self
            .entries
            .get(id as usize)
            .ok_or_else(|| LpcError::RuntimeError(format!("unknown kfun id: {}", id)))?;
        let argc = args.len() as u8;
        if argc < entry.min_args {
            return Err(LpcError::RuntimeError(format!(
                "{}(): too few arguments, expected at least {}, got {}",
                entry.name, entry.min_args, argc
            )));
        }
        if argc > entry.max_args {
            return Err(LpcError::RuntimeError(format!(
                "{}(): too many arguments, expected at most {}, got {}",
                entry.name, entry.max_args, argc
            )));
        }
        (entry.func)(ctx, args)
    }

    /// Return all registered kfun names.
    pub fn names(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.name.as_str()).collect()
    }

    /// Register all built-in DGD kernel functions.
    pub fn register_defaults(&mut self) {
        // Type inspection
        self.register("typeof", type_ops::kf_typeof, 1, 1);
        self.register("instanceof", type_ops::kf_instanceof, 2, 2);

        // String operations
        self.register("strlen", string::kf_strlen, 1, 1);
        self.register("explode", string::kf_explode, 2, 2);
        self.register("implode", string::kf_implode, 2, 2);
        self.register("lower_case", string::kf_lower_case, 1, 1);
        self.register("upper_case", string::kf_upper_case, 1, 1);
        self.register("sscanf", string::kf_sscanf, 2, 255);

        // Array operations
        self.register("allocate", array::kf_allocate, 1, 1);
        self.register("allocate_int", array::kf_allocate_int, 1, 1);
        self.register("allocate_float", array::kf_allocate_float, 1, 1);
        self.register("sizeof", array::kf_sizeof, 1, 1);
        self.register("sort_array", array::kf_sort_array, 2, 2);

        // Mapping operations
        self.register("map_indices", mapping::kf_map_indices, 1, 1);
        self.register("map_values", mapping::kf_map_values, 1, 1);
        self.register("map_sizeof", mapping::kf_map_sizeof, 1, 1);
        self.register("mkmapping", mapping::kf_mkmapping, 2, 2);

        // Math operations
        self.register("fabs", math::kf_fabs, 1, 1);
        self.register("floor", math::kf_floor, 1, 1);
        self.register("ceil", math::kf_ceil, 1, 1);
        self.register("sqrt", math::kf_sqrt, 1, 1);
        self.register("exp", math::kf_exp, 1, 1);
        self.register("log", math::kf_log, 1, 1);
        self.register("log10", math::kf_log10, 1, 1);
        self.register("sin", math::kf_sin, 1, 1);
        self.register("cos", math::kf_cos, 1, 1);
        self.register("tan", math::kf_tan, 1, 1);
        self.register("asin", math::kf_asin, 1, 1);
        self.register("acos", math::kf_acos, 1, 1);
        self.register("atan", math::kf_atan, 1, 1);
        self.register("sinh", math::kf_sinh, 1, 1);
        self.register("cosh", math::kf_cosh, 1, 1);
        self.register("tanh", math::kf_tanh, 1, 1);
        self.register("pow", math::kf_pow, 2, 2);
        self.register("fmod", math::kf_fmod, 2, 2);
        self.register("atan2", math::kf_atan2, 2, 2);
        self.register("ldexp", math::kf_ldexp, 2, 2);
        self.register("frexp", math::kf_frexp, 1, 1);
        self.register("modf", math::kf_modf, 1, 1);
        self.register("random", math::kf_random, 1, 1);

        // Object management
        self.register("this_object", object::kf_this_object, 0, 0);
        self.register("previous_object", object::kf_previous_object, 0, 1);
        self.register("clone_object", object::kf_clone_object, 1, 1);
        self.register("new_object", object::kf_new_object, 1, 1);
        self.register("destruct_object", object::kf_destruct_object, 1, 1);
        self.register("find_object", object::kf_find_object, 1, 1);
        self.register("object_name", object::kf_object_name, 1, 1);
        self.register("function_object", object::kf_function_object, 2, 2);
        self.register("compile_object", object::kf_compile_object, 1, 255);
        self.register("this_user", object::kf_this_user, 0, 0);

        // Timing and scheduling
        self.register("time", timing::kf_time, 0, 0);
        self.register("millitime", timing::kf_millitime, 0, 0);
        self.register("ctime", timing::kf_ctime, 1, 1);
        self.register("call_out", timing::kf_call_out, 2, 255);
        self.register("remove_call_out", timing::kf_remove_call_out, 1, 1);

        // File I/O (driver service stubs)
        self.register("read_file", io::kf_read_file, 1, 3);
        self.register("write_file", io::kf_write_file, 2, 3);
        self.register("remove_file", io::kf_remove_file, 1, 1);
        self.register("rename_file", io::kf_rename_file, 2, 2);
        self.register("get_dir", io::kf_get_dir, 1, 1);
        self.register("make_dir", io::kf_make_dir, 1, 1);
        self.register("remove_dir", io::kf_remove_dir, 1, 1);

        // Connection/communication (driver service stubs)
        self.register("send_message", connection::kf_send_message, 1, 1);
        self.register("users", connection::kf_users, 0, 0);
        self.register("query_ip_number", connection::kf_query_ip_number, 1, 1);
        self.register("query_ip_name", connection::kf_query_ip_name, 1, 1);

        // Serialization
        self.register("save_object", serialize::kf_save_object, 1, 1);
        self.register("restore_object", serialize::kf_restore_object, 1, 1);

        // Crypto/hashing
        self.register("crypt", crypto::kf_crypt, 2, 2);
        self.register("hash_crc16", crypto::kf_hash_crc16, 1, 255);
        self.register("hash_crc32", crypto::kf_hash_crc32, 1, 255);
        self.register("hash_string", crypto::kf_hash_string, 2, 2);

        // ASN (arbitrary-size number) operations
        self.register("asn_add", asn::kf_asn_add, 3, 3);
        self.register("asn_sub", asn::kf_asn_sub, 3, 3);
        self.register("asn_mult", asn::kf_asn_mult, 3, 3);
        self.register("asn_div", asn::kf_asn_div, 3, 3);
        self.register("asn_mod", asn::kf_asn_mod, 2, 2);
        self.register("asn_pow", asn::kf_asn_pow, 3, 3);
        self.register("asn_modinv", asn::kf_asn_modinv, 2, 2);
        self.register("asn_lshift", asn::kf_asn_lshift, 3, 3);
        self.register("asn_rshift", asn::kf_asn_rshift, 2, 2);
        self.register("asn_and", asn::kf_asn_and, 2, 2);
        self.register("asn_or", asn::kf_asn_or, 2, 2);
        self.register("asn_xor", asn::kf_asn_xor, 2, 2);
        self.register("asn_cmp", asn::kf_asn_cmp, 2, 2);

        // Connection/networking (additional driver service stubs)
        self.register("connect", connection::kf_connect, 2, 3);
        self.register("connect_datagram", connection::kf_connect_datagram, 3, 4);
        self.register("datagram_challenge", connection::kf_datagram_challenge, 1, 1);
        self.register("send_close", connection::kf_send_close, 0, 0);
        self.register("send_datagram", connection::kf_send_datagram, 1, 1);
        self.register("block_input", connection::kf_block_input, 1, 1);

        // Object management (additional)
        self.register("call_other", object::kf_call_other, 2, 255);
        self.register("call_touch", object::kf_call_touch, 1, 1);
        self.register("previous_program", object::kf_previous_program, 0, 1);

        // Timing (additional)
        self.register("call_out_summand", timing::kf_call_out_summand, 2, 255);

        // Editor (DGD-specific)
        self.register("editor", editor::kf_editor, 0, 1);
        self.register("query_editor", editor::kf_query_editor, 1, 1);

        // Parsing
        self.register("parse_string", parse::kf_parse_string, 2, 3);

        // Crypto (additional)
        self.register("encrypt", crypto::kf_encrypt, 2, 255);
        self.register("decrypt", crypto::kf_decrypt, 2, 255);

        // System
        self.register("swapout", misc::kf_swapout, 0, 0);

        // Miscellaneous
        self.register("error", misc::kf_error, 1, 1);
        self.register("call_trace", misc::kf_call_trace, 0, 0);
        self.register("status", misc::kf_status, 0, 1);
        self.register("dump_state", misc::kf_dump_state, 0, 1);
        self.register("shutdown", misc::kf_shutdown, 0, 1);
    }
}

impl Default for KfunRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper to extract an int from an LpcValue, returning a typed error.
pub(crate) fn require_int(val: &LpcValue, arg_pos: usize) -> Result<i64, LpcError> {
    val.as_int().ok_or_else(|| LpcError::TypeError {
        expected: "int",
        got: val.type_name().to_string(),
        arg_pos,
    })
}

/// Helper to extract a float from an LpcValue, returning a typed error.
pub(crate) fn require_float(val: &LpcValue, arg_pos: usize) -> Result<f64, LpcError> {
    val.as_float().ok_or_else(|| LpcError::TypeError {
        expected: "float",
        got: val.type_name().to_string(),
        arg_pos,
    })
}

/// Helper to extract a string from an LpcValue, returning a typed error.
pub(crate) fn require_string(val: &LpcValue, arg_pos: usize) -> Result<&str, LpcError> {
    val.as_string().ok_or_else(|| LpcError::TypeError {
        expected: "string",
        got: val.type_name().to_string(),
        arg_pos,
    })
}

/// Helper to extract an array from an LpcValue, returning a typed error.
pub(crate) fn require_array(val: &LpcValue, arg_pos: usize) -> Result<&[LpcValue], LpcError> {
    val.as_array().ok_or_else(|| LpcError::TypeError {
        expected: "array",
        got: val.type_name().to_string(),
        arg_pos,
    })
}

/// Helper to extract a mapping from an LpcValue, returning a typed error.
pub(crate) fn require_mapping(
    val: &LpcValue,
    arg_pos: usize,
) -> Result<&[(LpcValue, LpcValue)], LpcError> {
    val.as_mapping().ok_or_else(|| LpcError::TypeError {
        expected: "mapping",
        got: val.type_name().to_string(),
        arg_pos,
    })
}

/// Helper to extract an object ref from an LpcValue, returning a typed error.
pub(crate) fn require_object(val: &LpcValue, arg_pos: usize) -> Result<&ObjectRef, LpcError> {
    val.as_object().ok_or_else(|| LpcError::TypeError {
        expected: "object",
        got: val.type_name().to_string(),
        arg_pos,
    })
}
