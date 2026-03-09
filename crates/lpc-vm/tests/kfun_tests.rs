use lpc_vm::bytecode::{LpcValue, ObjectRef};
use lpc_vm::kfun::{
    KfunContext, KfunRegistry,
    T_ARRAY, T_FLOAT, T_INT, T_LWOBJECT, T_MAPPING, T_NIL, T_OBJECT, T_STRING,
};

/// Create a registry with all defaults and call a kfun by name.
fn call_kfun(name: &str, args: &[LpcValue]) -> Result<LpcValue, String> {
    let mut registry = KfunRegistry::new();
    registry.register_defaults();
    let id = registry.lookup(name).ok_or_else(|| format!("kfun '{}' not found", name))?;
    let mut ticks: u64 = 1_000_000;
    let obj = ObjectRef {
        id: 0,
        path: "/test".to_string(),
        is_lightweight: false,
    };
    let mut ctx = KfunContext {
        this_object: &obj,
        previous_object: None,
        tick_counter: &mut ticks,
    };
    registry.call(id, &mut ctx, args).map_err(|e| format!("{e}"))
}

// =========================================================================
// typeof
// =========================================================================

#[test]
fn typeof_nil() {
    let result = call_kfun("typeof", &[LpcValue::Nil]).unwrap();
    assert_eq!(result.as_int(), Some(T_NIL));
}

#[test]
fn typeof_int() {
    let result = call_kfun("typeof", &[LpcValue::Int(42)]).unwrap();
    assert_eq!(result.as_int(), Some(T_INT));
}

#[test]
fn typeof_float() {
    let result = call_kfun("typeof", &[LpcValue::Float(3.14)]).unwrap();
    assert_eq!(result.as_int(), Some(T_FLOAT));
}

#[test]
fn typeof_string() {
    let result = call_kfun("typeof", &[LpcValue::String("hi".into())]).unwrap();
    assert_eq!(result.as_int(), Some(T_STRING));
}

#[test]
fn typeof_object() {
    let obj = LpcValue::Object(ObjectRef {
        id: 1,
        path: "/test".into(),
        is_lightweight: false,
    });
    let result = call_kfun("typeof", &[obj]).unwrap();
    assert_eq!(result.as_int(), Some(T_OBJECT));
}

#[test]
fn typeof_array() {
    let result = call_kfun("typeof", &[LpcValue::Array(vec![])]).unwrap();
    assert_eq!(result.as_int(), Some(T_ARRAY));
}

#[test]
fn typeof_mapping() {
    let result = call_kfun("typeof", &[LpcValue::Mapping(vec![])]).unwrap();
    assert_eq!(result.as_int(), Some(T_MAPPING));
}

#[test]
fn typeof_lwobject() {
    let obj = LpcValue::Object(ObjectRef {
        id: 1,
        path: "/test".into(),
        is_lightweight: true,
    });
    let result = call_kfun("typeof", &[obj]).unwrap();
    assert_eq!(result.as_int(), Some(T_LWOBJECT));
}

// =========================================================================
// String operations
// =========================================================================

#[test]
fn strlen_basic() {
    let result = call_kfun("strlen", &[LpcValue::String("hello".into())]).unwrap();
    assert_eq!(result.as_int(), Some(5));
}

#[test]
fn strlen_empty() {
    let result = call_kfun("strlen", &[LpcValue::String("".into())]).unwrap();
    assert_eq!(result.as_int(), Some(0));
}

#[test]
fn explode_basic() {
    let result = call_kfun(
        "explode",
        &[
            LpcValue::String("a,b,c".into()),
            LpcValue::String(",".into()),
        ],
    )
    .unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0].as_string(), Some("a"));
    assert_eq!(arr[1].as_string(), Some("b"));
    assert_eq!(arr[2].as_string(), Some("c"));
}

#[test]
fn explode_empty_separator_splits_chars() {
    let result = call_kfun(
        "explode",
        &[
            LpcValue::String("abc".into()),
            LpcValue::String("".into()),
        ],
    )
    .unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0].as_string(), Some("a"));
    assert_eq!(arr[1].as_string(), Some("b"));
    assert_eq!(arr[2].as_string(), Some("c"));
}

#[test]
fn implode_basic() {
    let arr = LpcValue::Array(vec![
        LpcValue::String("hello".into()),
        LpcValue::String("world".into()),
    ]);
    let result = call_kfun("implode", &[arr, LpcValue::String(" ".into())]).unwrap();
    assert_eq!(result.as_string(), Some("hello world"));
}

#[test]
fn lower_case() {
    let result = call_kfun("lower_case", &[LpcValue::String("HELLO".into())]).unwrap();
    assert_eq!(result.as_string(), Some("hello"));
}

#[test]
fn upper_case() {
    let result = call_kfun("upper_case", &[LpcValue::String("hello".into())]).unwrap();
    assert_eq!(result.as_string(), Some("HELLO"));
}

// =========================================================================
// Array operations
// =========================================================================

#[test]
fn allocate_creates_nil_array() {
    let result = call_kfun("allocate", &[LpcValue::Int(3)]).unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert!(arr.iter().all(|v| *v == LpcValue::Nil));
}

#[test]
fn allocate_negative_size_error() {
    let result = call_kfun("allocate", &[LpcValue::Int(-1)]);
    assert!(result.is_err());
}

#[test]
fn sizeof_array() {
    let arr = LpcValue::Array(vec![LpcValue::Int(1), LpcValue::Int(2)]);
    let result = call_kfun("sizeof", &[arr]).unwrap();
    assert_eq!(result.as_int(), Some(2));
}

#[test]
fn sizeof_mapping() {
    let m = LpcValue::Mapping(vec![
        (LpcValue::String("a".into()), LpcValue::Int(1)),
    ]);
    let result = call_kfun("sizeof", &[m]).unwrap();
    assert_eq!(result.as_int(), Some(1));
}

#[test]
fn sizeof_string() {
    let result = call_kfun("sizeof", &[LpcValue::String("abc".into())]).unwrap();
    assert_eq!(result.as_int(), Some(3));
}

#[test]
fn sizeof_other_returns_zero() {
    let result = call_kfun("sizeof", &[LpcValue::Int(42)]).unwrap();
    assert_eq!(result.as_int(), Some(0));
}

// =========================================================================
// Mapping operations
// =========================================================================

#[test]
fn map_indices() {
    let m = LpcValue::Mapping(vec![
        (LpcValue::String("a".into()), LpcValue::Int(1)),
        (LpcValue::String("b".into()), LpcValue::Int(2)),
    ]);
    let result = call_kfun("map_indices", &[m]).unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    // Check keys are present (order may vary)
    let keys: Vec<&str> = arr.iter().map(|v| v.as_string().unwrap()).collect();
    assert!(keys.contains(&"a"));
    assert!(keys.contains(&"b"));
}

#[test]
fn map_values() {
    let m = LpcValue::Mapping(vec![
        (LpcValue::String("a".into()), LpcValue::Int(1)),
        (LpcValue::String("b".into()), LpcValue::Int(2)),
    ]);
    let result = call_kfun("map_values", &[m]).unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    let vals: Vec<i64> = arr.iter().map(|v| v.as_int().unwrap()).collect();
    assert!(vals.contains(&1));
    assert!(vals.contains(&2));
}

#[test]
fn mkmapping_creates_mapping() {
    let keys = LpcValue::Array(vec![
        LpcValue::String("x".into()),
        LpcValue::String("y".into()),
    ]);
    let vals = LpcValue::Array(vec![LpcValue::Int(10), LpcValue::Int(20)]);
    let result = call_kfun("mkmapping", &[keys, vals]).unwrap();
    let m = result.as_mapping().unwrap();
    assert_eq!(m.len(), 2);
}

#[test]
fn mkmapping_mismatched_lengths_error() {
    let keys = LpcValue::Array(vec![LpcValue::String("a".into())]);
    let vals = LpcValue::Array(vec![LpcValue::Int(1), LpcValue::Int(2)]);
    let result = call_kfun("mkmapping", &[keys, vals]);
    assert!(result.is_err());
}

// =========================================================================
// Math operations
// =========================================================================

#[test]
fn sqrt_of_four() {
    let result = call_kfun("sqrt", &[LpcValue::Float(4.0)]).unwrap();
    let f = result.as_float().unwrap();
    assert!((f - 2.0).abs() < 1e-10);
}

#[test]
fn sqrt_negative_error() {
    let result = call_kfun("sqrt", &[LpcValue::Float(-1.0)]);
    assert!(result.is_err());
}

#[test]
fn sin_zero() {
    let result = call_kfun("sin", &[LpcValue::Float(0.0)]).unwrap();
    let f = result.as_float().unwrap();
    assert!(f.abs() < 1e-10);
}

#[test]
fn cos_zero() {
    let result = call_kfun("cos", &[LpcValue::Float(0.0)]).unwrap();
    let f = result.as_float().unwrap();
    assert!((f - 1.0).abs() < 1e-10);
}

#[test]
fn random_range() {
    let result = call_kfun("random", &[LpcValue::Int(100)]).unwrap();
    let val = result.as_int().unwrap();
    assert!(val >= 0 && val < 100);
}

#[test]
fn random_negative_error() {
    let result = call_kfun("random", &[LpcValue::Int(-1)]);
    assert!(result.is_err());
}

#[test]
fn fabs_negative() {
    let result = call_kfun("fabs", &[LpcValue::Float(-3.14)]).unwrap();
    let f = result.as_float().unwrap();
    assert!((f - 3.14).abs() < 1e-10);
}

#[test]
fn floor_value() {
    let result = call_kfun("floor", &[LpcValue::Float(3.7)]).unwrap();
    let f = result.as_float().unwrap();
    assert!((f - 3.0).abs() < 1e-10);
}

#[test]
fn ceil_value() {
    let result = call_kfun("ceil", &[LpcValue::Float(3.2)]).unwrap();
    let f = result.as_float().unwrap();
    assert!((f - 4.0).abs() < 1e-10);
}

#[test]
fn pow_two_three() {
    let result = call_kfun("pow", &[LpcValue::Float(2.0), LpcValue::Float(3.0)]).unwrap();
    let f = result.as_float().unwrap();
    assert!((f - 8.0).abs() < 1e-10);
}

// =========================================================================
// Timing
// =========================================================================

#[test]
fn time_returns_positive() {
    let result = call_kfun("time", &[]).unwrap();
    let t = result.as_int().unwrap();
    assert!(t > 0);
}

// =========================================================================
// Crypto / hashing
// =========================================================================

#[test]
fn hash_crc32_consistent() {
    let result1 = call_kfun("hash_crc32", &[LpcValue::String("hello".into())]).unwrap();
    let result2 = call_kfun("hash_crc32", &[LpcValue::String("hello".into())]).unwrap();
    assert_eq!(result1, result2);
}

#[test]
fn hash_crc32_different_inputs() {
    let result1 = call_kfun("hash_crc32", &[LpcValue::String("hello".into())]).unwrap();
    let result2 = call_kfun("hash_crc32", &[LpcValue::String("world".into())]).unwrap();
    assert_ne!(result1, result2);
}

#[test]
fn hash_crc16_consistent() {
    let result1 = call_kfun("hash_crc16", &[LpcValue::String("test".into())]).unwrap();
    let result2 = call_kfun("hash_crc16", &[LpcValue::String("test".into())]).unwrap();
    assert_eq!(result1, result2);
}

// =========================================================================
// Miscellaneous
// =========================================================================

#[test]
fn error_kfun_returns_err() {
    let result = call_kfun("error", &[LpcValue::String("boom".into())]);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("boom"));
}

// =========================================================================
// Serialization round-trip
// =========================================================================

#[test]
fn serialize_round_trip_int() {
    use lpc_vm::kfun::serialize::{serialize_variables, parse_saved_variables};
    let vars = vec![("count".to_string(), LpcValue::Int(42))];
    let text = serialize_variables(&vars);
    let parsed = parse_saved_variables(&text).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].0, "count");
    assert_eq!(parsed[0].1, LpcValue::Int(42));
}

#[test]
fn serialize_round_trip_string() {
    use lpc_vm::kfun::serialize::{serialize_variables, parse_saved_variables};
    let vars = vec![("name".to_string(), LpcValue::String("hello \"world\"".into()))];
    let text = serialize_variables(&vars);
    let parsed = parse_saved_variables(&text).unwrap();
    assert_eq!(parsed[0].0, "name");
    assert_eq!(parsed[0].1, LpcValue::String("hello \"world\"".into()));
}

#[test]
fn serialize_round_trip_array() {
    use lpc_vm::kfun::serialize::{serialize_variables, parse_saved_variables};
    let arr = LpcValue::Array(vec![LpcValue::Int(1), LpcValue::Int(2), LpcValue::Int(3)]);
    let vars = vec![("data".to_string(), arr.clone())];
    let text = serialize_variables(&vars);
    let parsed = parse_saved_variables(&text).unwrap();
    assert_eq!(parsed[0].1, arr);
}

#[test]
fn serialize_round_trip_mapping() {
    use lpc_vm::kfun::serialize::{serialize_variables, parse_saved_variables};
    let m = LpcValue::Mapping(vec![
        (LpcValue::String("key".into()), LpcValue::Int(42)),
    ]);
    let vars = vec![("table".to_string(), m.clone())];
    let text = serialize_variables(&vars);
    let parsed = parse_saved_variables(&text).unwrap();
    assert_eq!(parsed[0].1, m);
}

#[test]
fn serialize_round_trip_nil() {
    use lpc_vm::kfun::serialize::{serialize_variables, parse_saved_variables};
    let vars = vec![("empty".to_string(), LpcValue::Nil)];
    let text = serialize_variables(&vars);
    let parsed = parse_saved_variables(&text).unwrap();
    assert_eq!(parsed[0].1, LpcValue::Nil);
}

#[test]
fn serialize_round_trip_float() {
    use lpc_vm::kfun::serialize::{serialize_variables, parse_saved_variables};
    let vars = vec![("pi".to_string(), LpcValue::Float(3.14))];
    let text = serialize_variables(&vars);
    let parsed = parse_saved_variables(&text).unwrap();
    assert_eq!(parsed[0].0, "pi");
    if let LpcValue::Float(f) = &parsed[0].1 {
        assert!((f - 3.14).abs() < 0.001);
    } else {
        panic!("expected float");
    }
}

// =========================================================================
// KfunRegistry
// =========================================================================

#[test]
fn registry_lookup() {
    let mut registry = KfunRegistry::new();
    registry.register_defaults();
    assert!(registry.lookup("typeof").is_some());
    assert!(registry.lookup("strlen").is_some());
    assert!(registry.lookup("nonexistent").is_none());
}

#[test]
fn registry_names() {
    let mut registry = KfunRegistry::new();
    registry.register_defaults();
    let names = registry.names();
    assert!(names.contains(&"typeof"));
    assert!(names.contains(&"strlen"));
    assert!(names.contains(&"sqrt"));
    assert!(names.contains(&"time"));
}

// =========================================================================
// allocate_int and allocate_float
// =========================================================================

#[test]
fn allocate_int_fills_zeros() {
    let result = call_kfun("allocate_int", &[LpcValue::Int(3)]).unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert!(arr.iter().all(|v| *v == LpcValue::Int(0)));
}

#[test]
fn allocate_float_fills_zeros() {
    let result = call_kfun("allocate_float", &[LpcValue::Int(3)]).unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert!(arr.iter().all(|v| *v == LpcValue::Float(0.0)));
}

// =========================================================================
// map_sizeof
// =========================================================================

#[test]
fn map_sizeof_basic() {
    let m = LpcValue::Mapping(vec![
        (LpcValue::String("a".into()), LpcValue::Int(1)),
        (LpcValue::String("b".into()), LpcValue::Int(2)),
    ]);
    let result = call_kfun("map_sizeof", &[m]).unwrap();
    assert_eq!(result.as_int(), Some(2));
}

// =========================================================================
// hash_string
// =========================================================================

#[test]
fn hash_string_in_range() {
    let result = call_kfun(
        "hash_string",
        &[LpcValue::String("test".into()), LpcValue::Int(100)],
    )
    .unwrap();
    let val = result.as_int().unwrap();
    assert!(val >= 0 && val < 100);
}

// =========================================================================
// sort_array
// =========================================================================

#[test]
fn sort_array_ints() {
    let arr = LpcValue::Array(vec![
        LpcValue::Int(3),
        LpcValue::Int(1),
        LpcValue::Int(2),
    ]);
    let result = call_kfun(
        "sort_array",
        &[arr, LpcValue::String("cmp".into())],
    )
    .unwrap();
    let sorted = result.as_array().unwrap();
    assert_eq!(sorted[0].as_int(), Some(1));
    assert_eq!(sorted[1].as_int(), Some(2));
    assert_eq!(sorted[2].as_int(), Some(3));
}
