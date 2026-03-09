# LPC Kfun Implementation Plan

> **For Claude:** This is a companion to `2026-03-09-rust-lpc-adapter-plan.md` Task 13.
> Reference the DGD source at `../dgd/src/kfun/` for exact semantics.

**Goal:** Implement all 117+ DGD kernel functions in Rust, organized by category,
with full behavioral compatibility including edge cases and error handling.

**Architecture:** Kfuns are registered in a `KfunRegistry` that maps names to Rust
function pointers. The registry is extensible — the stdlib layer can register
additional kfuns. Kfuns that require driver services (file I/O, networking,
compilation) go through the `DriverServices` trait (implemented via MOP in the adapter).

---

## Kfun Calling Convention

```rust
/// Context provided to every kfun call.
pub struct VmContext<'a> {
    pub vm: &'a mut Vm,
    pub this_object: ObjectRef,
    pub previous_object: Option<ObjectRef>,
    pub tick_counter: &'a mut u64,
    pub driver_services: &'a dyn DriverServices,
}

/// Every kfun has this signature.
pub type KfunFn = fn(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError>;

/// Registry mapping kfun names to implementations.
pub struct KfunRegistry {
    by_name: HashMap<String, u16>,
    by_id: Vec<(String, KfunFn)>,
}

impl KfunRegistry {
    pub fn register(&mut self, name: &str, f: KfunFn) -> u16;
    pub fn lookup(&self, name: &str) -> Option<u16>;
    pub fn call(&self, id: u16, ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError>;
    pub fn register_defaults(&mut self);  // registers all built-in kfuns
}
```

**Error handling:** Kfuns return `Result<LpcValue, LpcError>`. LpcError variants:
- `TypeError { expected, got, arg_pos }` — wrong argument type
- `ValueError(String)` — invalid value (division by zero, out of range, etc.)
- `RuntimeError(String)` — general runtime error
- `AtomicViolation(String)` — operation not allowed in atomic context

**Tick costs:** Each kfun specifies its tick cost. The VM deducts ticks before execution.
DGD's actual tick costs are documented per-kfun below.

---

## Type Constants

Used by `typeof()` and type-checking kfuns:

```rust
pub const T_NIL: i64 = 0;
pub const T_INT: i64 = 1;
pub const T_FLOAT: i64 = 2;
pub const T_STRING: i64 = 3;
pub const T_OBJECT: i64 = 4;
pub const T_ARRAY: i64 = 5;
pub const T_MAPPING: i64 = 6;
pub const T_LWOBJECT: i64 = 7;
```

---

## Category 1: Type Inspection

**File:** `crates/lpc-vm/src/kfun/type_ops.rs`
**Ticks:** 1 per call unless noted

### typeof(mixed value) → int

Returns the type constant for the given value.

```rust
fn kf_typeof(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
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
```

**Tests:**
```rust
#[test]
fn typeof_int() { assert_kfun_eq("typeof", &[int(42)], int(T_INT)); }
#[test]
fn typeof_nil() { assert_kfun_eq("typeof", &[nil()], int(T_NIL)); }
#[test]
fn typeof_string() { assert_kfun_eq("typeof", &[str("hi")], int(T_STRING)); }
#[test]
fn typeof_array() { assert_kfun_eq("typeof", &[array(vec![])], int(T_ARRAY)); }
#[test]
fn typeof_mapping() { assert_kfun_eq("typeof", &[mapping(vec![])], int(T_MAPPING)); }
```

### instanceof(object obj, string type_name) → int

Checks if an object's program inherits from the named type. Resolves the type name
through the driver object's `object_type()` function.

```rust
fn kf_instanceof(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let obj = args[0].as_object()?;
    let type_name = args[1].as_string()?;
    let program = ctx.vm.get_program(&obj)?;
    let result = program.inherits_from(type_name);
    Ok(LpcValue::Int(if result { 1 } else { 0 }))
}
```

**Edge cases:**
- Destructed object → error
- Lightweight object → check its master's inheritance

---

## Category 2: String Operations

**File:** `crates/lpc-vm/src/kfun/string.rs`

### strlen(string s) → int

**Ticks:** 1

```rust
fn kf_strlen(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s = args[0].as_string()?;
    Ok(LpcValue::Int(s.len() as i64))
}
```

### explode(string s, string separator) → string*

Split string by separator. If separator is empty string `""`, splits into individual
characters. Leading/trailing separators do NOT produce empty strings (DGD behavior).

**Ticks:** `1 + result_length`

```rust
fn kf_explode(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s = args[0].as_string()?;
    let sep = args[1].as_string()?;

    let parts = if sep.is_empty() {
        // Split into individual characters
        s.chars().map(|c| LpcValue::String(c.to_string())).collect()
    } else {
        // Split by separator, filter out empty strings at edges
        s.split(sep)
            .filter(|p| !p.is_empty())
            .map(|p| LpcValue::String(p.to_string()))
            .collect()
    };

    ctx.tick_counter.saturating_sub(parts.len() as u64);
    Ok(LpcValue::Array(parts))
}
```

**Tests:**
```rust
#[test]
fn explode_simple() {
    assert_kfun_eq("explode", &[str("a:b:c"), str(":")],
        array(vec![str("a"), str("b"), str("c")]));
}
#[test]
fn explode_empty_sep() {
    assert_kfun_eq("explode", &[str("abc"), str("")],
        array(vec![str("a"), str("b"), str("c")]));
}
#[test]
fn explode_leading_sep() {
    // DGD strips leading/trailing empty strings
    assert_kfun_eq("explode", &[str(":a:b:"), str(":")],
        array(vec![str("a"), str("b")]));
}
#[test]
fn explode_no_match() {
    assert_kfun_eq("explode", &[str("abc"), str("x")],
        array(vec![str("abc")]));
}
```

### implode(string* arr, string separator) → string

Join array of strings with separator. Error if any element is not a string.

**Ticks:** `1 + array_length`

```rust
fn kf_implode(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let arr = args[0].as_array()?;
    let sep = args[1].as_string()?;

    let mut parts = Vec::with_capacity(arr.len());
    for (i, v) in arr.iter().enumerate() {
        match v {
            LpcValue::String(s) => parts.push(s.as_str()),
            _ => return Err(LpcError::TypeError {
                expected: "string",
                got: v.type_name(),
                arg_pos: i,
            }),
        }
    }

    ctx.tick_counter.saturating_sub(arr.len() as u64);
    Ok(LpcValue::String(parts.join(sep)))
}
```

### lower_case(string s) → string

**Ticks:** `1 + len/2`

```rust
fn kf_lower_case(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s = args[0].as_string()?;
    ctx.tick_counter.saturating_sub(s.len() as u64 / 2);
    Ok(LpcValue::String(s.to_lowercase()))
}
```

### upper_case(string s) → string

**Ticks:** `1 + len/2`

```rust
fn kf_upper_case(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s = args[0].as_string()?;
    ctx.tick_counter.saturating_sub(s.len() as u64 / 2);
    Ok(LpcValue::String(s.to_uppercase()))
}
```

### sscanf(string input, string format, args...) → int

Formatted string scanning. Format specifiers:
- `%s` — match string (greedy, up to next literal or format)
- `%d` — match integer
- `%f` — match float
- `%c` — match single character
- `%*s`, `%*d`, etc. — match but don't assign

Returns number of successful matches.

**Ticks:** `8 per match`

```rust
fn kf_sscanf(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let input = args[0].as_string()?;
    let format = args[1].as_string()?;
    // Remaining args are lvalue references for assignment
    // Parse format string, match against input, assign to lvalues
    // Return count of successful matches
    todo!("Complex implementation — see DGD extra.cpp kf_sscanf")
}
```

**DGD behavior details** (from `extra.cpp`):
- `%s` followed by literal: matches everything up to the literal
- `%s` followed by `%d`: matches everything up to the first digit
- `%s` at end: matches rest of string
- `%d` matches optional sign + digits
- Returns number of matched format specifiers (not counting `%*`)

**Tests:**
```rust
#[test]
fn sscanf_simple() {
    // sscanf("hello world", "%s %s", &a, &b) → 2, a="hello", b="world"
}
#[test]
fn sscanf_int() {
    // sscanf("age: 42", "age: %d", &n) → 1, n=42
}
#[test]
fn sscanf_mixed() {
    // sscanf("item 5 gold", "item %d %s", &n, &s) → 2, n=5, s="gold"
}
```

---

## Category 3: Array Operations

**File:** `crates/lpc-vm/src/kfun/array.rs`

### allocate(int size) → mixed*

Create array of given size, all elements initialized to nil.

**Ticks:** `1 + size`

```rust
fn kf_allocate(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let size = args[0].as_int()?;
    if size < 0 {
        return Err(LpcError::ValueError("negative array size".into()));
    }
    ctx.tick_counter.saturating_sub(size as u64);
    Ok(LpcValue::Array(vec![LpcValue::Nil; size as usize]))
}
```

### allocate_int(int size) → int*

Create array of given size, all elements initialized to 0.

**Ticks:** `1 + size`

```rust
fn kf_allocate_int(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let size = args[0].as_int()?;
    if size < 0 {
        return Err(LpcError::ValueError("negative array size".into()));
    }
    ctx.tick_counter.saturating_sub(size as u64);
    Ok(LpcValue::Array(vec![LpcValue::Int(0); size as usize]))
}
```

### allocate_float(int size) → float*

Create array of given size, all elements initialized to 0.0.

**Ticks:** `1 + size`

### sizeof(mixed value) → int

Returns size of array, mapping, or string. For mappings, returns number of key-value
pairs. For other types, returns 0.

**Ticks:** 1

```rust
fn kf_sizeof(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let size = match &args[0] {
        LpcValue::Array(a) => a.len() as i64,
        LpcValue::Mapping(m) => m.len() as i64,
        LpcValue::String(s) => s.len() as i64,
        _ => 0,
    };
    Ok(LpcValue::Int(size))
}
```

### sort_array(mixed* arr, string compare_func) → mixed*

Sort array using a comparison function defined in the current object.
The compare function receives two elements and returns negative, zero, or positive.

**Ticks:** `5 + n*log(n)` (approximately)

```rust
fn kf_sort_array(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let mut arr = args[0].as_array()?.clone();
    let func_name = args[1].as_string()?;

    // Sort using the named comparison function
    // Each comparison calls func_name(a, b) in the current object
    // Returns a sorted copy (DGD sorts in-place but we return new array for safety)
    arr.sort_by(|a, b| {
        let result = ctx.vm.call_function(
            &ctx.this_object, func_name, &[a.clone(), b.clone()]
        );
        match result {
            Ok(LpcValue::Int(n)) if n < 0 => std::cmp::Ordering::Less,
            Ok(LpcValue::Int(0)) => std::cmp::Ordering::Equal,
            _ => std::cmp::Ordering::Greater,
        }
    });

    Ok(LpcValue::Array(arr))
}
```

---

## Category 4: Mapping Operations

**File:** `crates/lpc-vm/src/kfun/mapping.rs`

### map_indices(mapping m) → mixed*

Returns array of all keys in the mapping.

**Ticks:** `1 + size`

```rust
fn kf_map_indices(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let m = args[0].as_mapping()?;
    ctx.tick_counter.saturating_sub(m.len() as u64);
    let keys: Vec<LpcValue> = m.iter().map(|(k, _)| k.clone()).collect();
    Ok(LpcValue::Array(keys))
}
```

### map_values(mapping m) → mixed*

Returns array of all values in the mapping.

**Ticks:** `1 + size`

```rust
fn kf_map_values(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let m = args[0].as_mapping()?;
    ctx.tick_counter.saturating_sub(m.len() as u64);
    let vals: Vec<LpcValue> = m.iter().map(|(_, v)| v.clone()).collect();
    Ok(LpcValue::Array(vals))
}
```

### map_sizeof(mapping m) → int

Returns number of key-value pairs. Same as `sizeof()` for mappings.

**Ticks:** 1

```rust
fn kf_map_sizeof(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let m = args[0].as_mapping()?;
    Ok(LpcValue::Int(m.len() as i64))
}
```

### mkmapping(mixed* keys, mixed* values) → mapping

Create a mapping from parallel arrays of keys and values. Arrays must be same length.

**Ticks:** `1 + length`

```rust
fn kf_mkmapping(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let keys = args[0].as_array()?;
    let vals = args[1].as_array()?;
    if keys.len() != vals.len() {
        return Err(LpcError::ValueError("key and value arrays must be same length".into()));
    }
    ctx.tick_counter.saturating_sub(keys.len() as u64);
    let pairs: Vec<(LpcValue, LpcValue)> = keys.iter()
        .zip(vals.iter())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    Ok(LpcValue::Mapping(pairs))
}
```

---

## Category 5: Math Operations

**File:** `crates/lpc-vm/src/kfun/math.rs`

All math kfuns operate on floats and return floats unless noted.
DGD's Float type uses custom IEEE 754 routines, but we use Rust's native f64.

### Single-argument float functions

Each follows the same pattern:

```rust
fn kf_mathfunc(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let x = args[0].as_float()?;
    ctx.tick_counter.saturating_sub(TICK_COST);
    let result = x.mathfunc();
    if result.is_nan() || result.is_infinite() {
        return Err(LpcError::ValueError("math domain error".into()));
    }
    Ok(LpcValue::Float(result))
}
```

| Function | Rust impl | Ticks | Notes |
|----------|-----------|-------|-------|
| `fabs(float)` | `f64::abs()` | 1 | Absolute value |
| `floor(float)` | `f64::floor()` | 1 | Round toward negative infinity |
| `ceil(float)` | `f64::ceil()` | 1 | Round toward positive infinity |
| `sqrt(float)` | `f64::sqrt()` | 11 | Error if negative |
| `exp(float)` | `f64::exp()` | 21 | e^x, error if overflow |
| `log(float)` | `f64::ln()` | 35 | Natural log, error if x ≤ 0 |
| `log10(float)` | `f64::log10()` | 41 | Log base 10, error if x ≤ 0 |
| `sin(float)` | `f64::sin()` | 25 | Sine |
| `cos(float)` | `f64::cos()` | 25 | Cosine |
| `tan(float)` | `f64::tan()` | 31 | Tangent |
| `asin(float)` | `f64::asin()` | 24 | Arcsine, error if \|x\| > 1 |
| `acos(float)` | `f64::acos()` | 24 | Arccosine, error if \|x\| > 1 |
| `atan(float)` | `f64::atan()` | 24 | Arctangent |
| `sinh(float)` | `f64::sinh()` | 24 | Hyperbolic sine |
| `cosh(float)` | `f64::cosh()` | 24 | Hyperbolic cosine |
| `tanh(float)` | `f64::tanh()` | 24 | Hyperbolic tangent |

### Two-argument float functions

| Function | Rust impl | Ticks | Notes |
|----------|-----------|-------|-------|
| `pow(float x, float y)` | `f64::powf()` | 48 | x^y, error on domain issues |
| `fmod(float x, float y)` | `f64::rem_euclid()` or custom | 1 | Float modulo, error if y=0 |
| `atan2(float y, float x)` | `f64::atan2()` | 27 | Two-argument arctangent |
| `ldexp(float x, int n)` | `f64::ldexp()` or `x * 2f64.powi(n)` | 1 | x × 2^n |

### Special float functions

#### frexp(float x) → mixed*

Split float into mantissa and exponent. Returns `({mantissa, exponent})` where
`x = mantissa × 2^exponent` and `0.5 ≤ |mantissa| < 1.0`.

**Ticks:** 2

```rust
fn kf_frexp(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let x = args[0].as_float()?;
    let (mantissa, exponent) = frexp(x);  // use libm::frexp or manual implementation
    Ok(LpcValue::Array(vec![
        LpcValue::Float(mantissa),
        LpcValue::Int(exponent as i64),
    ]))
}
```

#### modf(float x) → mixed*

Split float into integer and fractional parts. Returns `({integer_part, frac_part})`.

**Ticks:** 2

```rust
fn kf_modf(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let x = args[0].as_float()?;
    let int_part = x.trunc();
    let frac_part = x.fract();
    Ok(LpcValue::Array(vec![
        LpcValue::Float(int_part),
        LpcValue::Float(frac_part),
    ]))
}
```

### random(int range) → int

Generate random number.

**Ticks:** 1

```rust
fn kf_random(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let range = args[0].as_int()?;
    if range < 0 {
        return Err(LpcError::ValueError("negative random range".into()));
    }
    if range == 0 {
        // DGD: return full 63-bit random value
        let val: i64 = ctx.vm.rng.gen::<i64>().abs();
        return Ok(LpcValue::Int(val));
    }
    // Return 0..range-1
    let val = (ctx.vm.rng.gen::<u64>() % range as u64) as i64;
    Ok(LpcValue::Int(val))
}
```

**Tests:**
```rust
#[test]
fn random_zero_returns_large() {
    let r = call_kfun("random", &[int(0)]);
    assert!(r.as_int().unwrap() >= 0);
}
#[test]
fn random_range() {
    for _ in 0..100 {
        let r = call_kfun("random", &[int(10)]).as_int().unwrap();
        assert!(r >= 0 && r < 10);
    }
}
#[test]
fn random_negative_errors() {
    assert!(call_kfun("random", &[int(-1)]).is_err());
}
```

---

## Category 6: Object Management

**File:** `crates/lpc-vm/src/kfun/object.rs`

### this_object() → object

Returns the current object (the one whose function is executing).

**Ticks:** 1

```rust
fn kf_this_object(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    Ok(LpcValue::Object(ctx.this_object.clone()))
}
```

**Edge cases:**
- If current object is destructed, returns nil (DGD checks `count != 0`)

### previous_object(varargs int depth) → object

Returns the object that called the current function. With depth argument, walks
further back in the call stack.

**Ticks:** 1

```rust
fn kf_previous_object(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let depth = if args.is_empty() { 0 } else { args[0].as_int()? };
    if depth < 0 {
        return Err(LpcError::ValueError("negative depth".into()));
    }
    match ctx.vm.get_caller(depth as usize) {
        Some(obj_ref) => Ok(LpcValue::Object(obj_ref.clone())),
        None => Ok(LpcValue::Nil),
    }
}
```

### clone_object(object master) → object

Create a clone of a master object. Clones share the master's program but have
their own variable state. Clone names are `"master_path#N"` where N is sequential.

**Ticks:** 1 (plus ticks for `create()` call)

```rust
fn kf_clone_object(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let master = args[0].as_object()?;
    if master.is_lightweight {
        return Err(LpcError::RuntimeError("cannot clone lightweight object".into()));
    }
    if !ctx.vm.is_master(&master) {
        return Err(LpcError::RuntimeError("can only clone master objects".into()));
    }
    let clone = ctx.vm.clone_object(&master)?;
    // Call create() on the new clone if it exists
    if ctx.vm.has_function(&clone, "create") {
        ctx.vm.call_function(&clone, "create", &[])?;
    }
    Ok(LpcValue::Object(clone))
}
```

**Edge cases:**
- Cannot clone a clone (must be master object, `O_MASTER` flag)
- Cannot clone a lightweight object
- Cannot clone a destructed object

### new_object(object master) → object

Create a lightweight object. Lightweight objects are reference-counted and
automatically deallocated when no references remain. Named `"master_path#-1"`.

**Ticks:** 1

```rust
fn kf_new_object(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let master = args[0].as_object()?;
    let lwo = ctx.vm.new_lightweight_object(&master)?;
    if ctx.vm.has_function(&lwo, "create") {
        ctx.vm.call_function(&lwo, "create", &[])?;
    }
    Ok(LpcValue::Object(lwo))
}
```

### destruct_object(object obj) → void

Destroy an object, removing it from the object table. If the object has a user
connection, the connection is closed. All references to the object become nil.

**Ticks:** 1

```rust
fn kf_destruct_object(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let obj = args[0].as_object()?;
    ctx.vm.destruct_object(&obj)?;
    Ok(LpcValue::Nil)
}
```

### find_object(string path) → object

Find a compiled object by its path. Returns nil if the object hasn't been compiled
or has been destructed.

**Ticks:** 1

```rust
fn kf_find_object(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let path = args[0].as_string()?;
    match ctx.vm.find_object(path) {
        Some(obj_ref) => Ok(LpcValue::Object(obj_ref)),
        None => Ok(LpcValue::Nil),
    }
}
```

### object_name(object obj) → string

Returns the path name of an object. For clones, appends `#N`. For lightweight
objects, appends `#-1`.

**Ticks:** 1

```rust
fn kf_object_name(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let obj = args[0].as_object()?;
    Ok(LpcValue::String(ctx.vm.object_name(&obj)))
}
```

### function_object(string func, object obj) → string

Returns the path of the program that defines the named function in the given object.
Returns nil if the function doesn't exist or isn't callable.

**Ticks:** 1

```rust
fn kf_function_object(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let func = args[0].as_string()?;
    let obj = args[1].as_object()?;
    match ctx.vm.function_origin(&obj, func) {
        Some(path) => Ok(LpcValue::String(path)),
        None => Ok(LpcValue::Nil),
    }
}
```

### compile_object(string path, string... sources) → object

Compile (or recompile) an LPC source file. If the object already exists, recompiles
and triggers the upgrade chain. Optional extra source strings are prepended.

**Ticks:** varies (compilation cost)

```rust
fn kf_compile_object(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let path = args[0].as_string()?;
    // Read source via driver services (file I/O through MOP)
    let source = ctx.driver_services.read_file(path)?;
    // Compile and register the program
    let obj = ctx.vm.compile_object(path, &source)?;
    Ok(LpcValue::Object(obj))
}
```

**Edge cases:**
- Recompilation triggers `upgraded()` on dependents
- Not allowed in atomic functions

---

## Category 7: Timing and Scheduling

**File:** `crates/lpc-vm/src/kfun/timing.rs`

### time() → int

Returns current Unix timestamp (seconds since epoch).

**Ticks:** 1

```rust
fn kf_time(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    Ok(LpcValue::Int(now))
}
```

### millitime() → mixed*

Returns `({seconds, millisecond_fraction})` — the seconds as int and the
sub-second fraction as float (0.0 to 0.999...).

**Ticks:** 1

```rust
fn kf_millitime(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    let secs = now.as_secs() as i64;
    let frac = (now.subsec_millis() as f64) / 1000.0;
    Ok(LpcValue::Array(vec![
        LpcValue::Int(secs),
        LpcValue::Float(frac),
    ]))
}
```

### ctime(int timestamp) → string

Convert Unix timestamp to human-readable date string. Returns 24-character string
in the format: `"Mon Jan  1 00:00:00 2024"`.

**Ticks:** 5

```rust
fn kf_ctime(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let ts = args[0].as_int()?;
    // Format using chrono or manual formatting
    // DGD uses P_ctime which produces "Day Mon DD HH:MM:SS YYYY"
    let dt = chrono::DateTime::from_timestamp(ts, 0)
        .ok_or_else(|| LpcError::ValueError("invalid timestamp".into()))?;
    let formatted = dt.format("%a %b %e %H:%M:%S %Y").to_string();
    Ok(LpcValue::String(formatted))
}
```

### call_out(string func, mixed delay, args...) → int

Schedule a function call after a delay. Returns a handle for cancellation.

**Ticks:** 1

```rust
fn kf_call_out(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let func = args[0].as_string()?;
    let delay = match &args[1] {
        LpcValue::Int(secs) => Duration::from_secs(*secs as u64),
        LpcValue::Float(secs) => Duration::from_secs_f64(*secs),
        _ => return Err(LpcError::TypeError {
            expected: "int or float", got: args[1].type_name(), arg_pos: 1
        }),
    };
    let extra_args: Vec<LpcValue> = args[2..].to_vec();

    if ctx.this_object.is_lightweight {
        return Err(LpcError::RuntimeError("call_out not allowed on lightweight objects".into()));
    }
    if ctx.vm.is_atomic() {
        return Err(LpcError::AtomicViolation("call_out not allowed in atomic context".into()));
    }

    let handle = ctx.vm.scheduler.schedule(
        ctx.this_object.clone(),
        func.to_string(),
        extra_args,
        delay,
    );
    Ok(LpcValue::Int(handle as i64))
}
```

**Edge cases:**
- Not allowed in atomic functions
- Not allowed on lightweight objects
- Delay can be int (seconds) or float (seconds with millisecond precision)

### remove_call_out(int handle) → mixed

Cancel a pending call_out. Returns remaining delay (int or float), or nil if
the handle was not found.

**Ticks:** 1

```rust
fn kf_remove_call_out(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let handle = args[0].as_int()?;
    match ctx.vm.scheduler.cancel(handle as u64) {
        Some(remaining) => {
            let secs = remaining.as_secs_f64();
            if secs == secs.floor() {
                Ok(LpcValue::Int(secs as i64))
            } else {
                Ok(LpcValue::Float(secs))
            }
        }
        None => Ok(LpcValue::Nil),
    }
}
```

---

## Category 8: I/O and File Operations (Driver Service Kfuns)

**File:** `crates/lpc-vm/src/kfun/io.rs`

These kfuns route through `DriverServices` trait via MOP.

### read_file(string path, varargs int start, int lines) → string

Read file contents. Optional start line and line count.

**Ticks:** varies by file size

```rust
fn kf_read_file(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    if ctx.vm.is_atomic() {
        return Err(LpcError::AtomicViolation("read_file not allowed in atomic".into()));
    }
    let path = args[0].as_string()?;
    let start = args.get(1).map(|v| v.as_int()).transpose()?.unwrap_or(0);
    let lines = args.get(2).map(|v| v.as_int()).transpose()?.unwrap_or(0);

    let content = ctx.driver_services.read_file(path, start, lines)?;
    match content {
        Some(s) => Ok(LpcValue::String(s)),
        None => Ok(LpcValue::Nil),
    }
}
```

### write_file(string path, string content, varargs int offset) → int

Write content to file. Optional offset for appending at specific position.
Returns 1 on success.

**Ticks:** varies

```rust
fn kf_write_file(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    if ctx.vm.is_atomic() {
        return Err(LpcError::AtomicViolation("write_file not allowed in atomic".into()));
    }
    let path = args[0].as_string()?;
    let content = args[1].as_string()?;
    let offset = args.get(2).map(|v| v.as_int()).transpose()?;

    ctx.driver_services.write_file(path, content, offset)?;
    Ok(LpcValue::Int(1))
}
```

### remove_file(string path) → int

Delete a file. Returns 1 on success, 0 on failure.

### rename_file(string from, string to) → int

Rename/move a file. Returns 1 on success, 0 on failure.

### get_dir(string pattern) → mixed**

List directory contents matching pattern. Returns a 3-element array:
`({names, sizes, timestamps})` where each is an array.

**Ticks:** varies

### make_dir(string path) → int

Create a directory. Returns 1 on success, 0 on failure.

### remove_dir(string path) → int

Remove an empty directory. Returns 1 on success, 0 on failure.

### file_info(string path) → mixed*

Returns `({size, timestamp})` for a file, or nil if not found.

---

## Category 9: Connection/Communication (Driver Service Kfuns)

**File:** `crates/lpc-vm/src/kfun/connection.rs`

### send_message(mixed msg) → int

Send data to the current user's connection.
- If msg is string: send as text
- If msg is int: 0 = disable echo, 1 = enable echo

Returns number of bytes sent.

```rust
fn kf_send_message(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    match &args[0] {
        LpcValue::String(text) => {
            let bytes = ctx.driver_services.send_to_session(
                ctx.this_object.session_id()?,
                text,
            )?;
            Ok(LpcValue::Int(bytes as i64))
        }
        LpcValue::Int(flag) => {
            ctx.driver_services.set_echo(ctx.this_object.session_id()?, *flag != 0)?;
            Ok(LpcValue::Int(0))
        }
        _ => Err(LpcError::TypeError {
            expected: "string or int", got: args[0].type_name(), arg_pos: 0
        }),
    }
}
```

### users() → object*

Returns array of all connected user objects.

```rust
fn kf_users(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let user_objects = ctx.driver_services.list_users()?;
    Ok(LpcValue::Array(user_objects))
}
```

### query_ip_number(object user) → string

Returns IP address of a user's connection.

### query_ip_name(object user) → string

Returns hostname of a user's connection.

### this_user() → object

Returns the current user object (the one associated with the current execution
context). Returns nil if not in a user context.

```rust
fn kf_this_user(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    match ctx.vm.current_user() {
        Some(user_ref) => Ok(LpcValue::Object(user_ref)),
        None => Ok(LpcValue::Nil),
    }
}
```

---

## Category 10: Object Serialization

**File:** `crates/lpc-vm/src/kfun/serialize.rs`

### save_object(string path) → void

Serialize the current object's non-private, non-static variables to a file.
Format: one variable per line, `varname value\n`.

In our architecture, this goes through the driver's state store rather than
direct file I/O.

**Ticks:** varies

```rust
fn kf_save_object(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    if ctx.vm.is_atomic() {
        return Err(LpcError::AtomicViolation("save_object not allowed in atomic".into()));
    }
    let path = args[0].as_string()?;
    let vars = ctx.vm.get_saveable_variables(&ctx.this_object);
    let serialized = serialize_variables(&vars);
    ctx.driver_services.write_file(path, &serialized, None)?;
    Ok(LpcValue::Nil)
}

fn serialize_variables(vars: &[(String, LpcValue)]) -> String {
    let mut out = String::new();
    for (name, value) in vars {
        out.push_str(name);
        out.push(' ');
        serialize_value(&mut out, value);
        out.push('\n');
    }
    out
}

fn serialize_value(out: &mut String, value: &LpcValue) {
    match value {
        LpcValue::Nil => out.push_str("nil"),
        LpcValue::Int(n) => out.push_str(&n.to_string()),
        LpcValue::Float(f) => out.push_str(&format!("{:.6}", f)),
        LpcValue::String(s) => {
            out.push('"');
            for ch in s.chars() {
                match ch {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    _ => out.push(ch),
                }
            }
            out.push('"');
        }
        LpcValue::Array(arr) => {
            out.push_str("({");
            for (i, v) in arr.iter().enumerate() {
                if i > 0 { out.push(','); }
                serialize_value(out, v);
            }
            out.push_str("})");
        }
        LpcValue::Mapping(m) => {
            out.push_str("([");
            for (i, (k, v)) in m.iter().enumerate() {
                if i > 0 { out.push(','); }
                serialize_value(out, k);
                out.push(':');
                serialize_value(out, v);
            }
            out.push_str("])");
        }
        LpcValue::Object(obj) => out.push_str(&format!("<{}>", obj.path)),
    }
}
```

### restore_object(string path) → void

Read back serialized variables and set them on the current object.

```rust
fn kf_restore_object(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let path = args[0].as_string()?;
    let content = ctx.driver_services.read_file(path, 0, 0)?
        .ok_or_else(|| LpcError::RuntimeError(format!("file not found: {}", path)))?;
    let vars = parse_saved_variables(&content)?;
    for (name, value) in vars {
        ctx.vm.set_variable(&ctx.this_object, &name, value)?;
    }
    Ok(LpcValue::Nil)
}
```

---

## Category 11: Hash and Crypto

**File:** `crates/lpc-vm/src/kfun/crypto.rs`

### crypt(string password, string salt) → string

Unix-style password hashing (DES or bcrypt depending on salt format).

**Ticks:** 39

```rust
fn kf_crypt(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let password = args[0].as_string()?;
    let salt = args[1].as_string()?;
    // Use bcrypt or DES crypt depending on salt
    let result = unix_crypt(password, salt)?;
    Ok(LpcValue::String(result))
}
```

### hash_crc16(string data, ...) → int

CRC-16/CCITT checksum. Multiple string arguments are accumulated.

**Ticks:** `3*nargs + total_len/4`

```rust
fn kf_hash_crc16(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let mut crc: u16 = 0xFFFF;
    for arg in args {
        let data = arg.as_string()?;
        for byte in data.bytes() {
            crc = (crc << 8) ^ CRC16_TABLE[((crc >> 8) as u8 ^ byte) as usize];
        }
    }
    Ok(LpcValue::Int(crc as i64))
}
```

### hash_crc32(string data, ...) → int

CRC-32 checksum. Multiple string arguments are accumulated.

**Ticks:** `3*nargs + total_len/4`

```rust
fn kf_hash_crc32(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let mut crc: u32 = 0xFFFFFFFF;
    for arg in args {
        let data = arg.as_string()?;
        for byte in data.bytes() {
            crc = (crc >> 8) ^ CRC32_TABLE[(crc as u8 ^ byte) as usize];
        }
    }
    Ok(LpcValue::Int(!crc as i64))
}
```

### hash_string(string key, int table_size) → int

Hash string to integer in range `0..table_size-1`. Used for distributed hashing.

### encrypt(string data, string cipher, ...) → string

Symmetric encryption. The cipher name selects the algorithm. DGD supports DES by
default.

### decrypt(string data, string cipher, ...) → string

Symmetric decryption. Mirror of encrypt.

---

## Category 12: Arbitrary Precision Arithmetic (ASN)

**File:** `crates/lpc-vm/src/kfun/asn.rs`

All ASN functions operate on strings representing large integers in big-endian
binary format. Consider using the `num-bigint` crate.

| Function | Signature | Description |
|----------|-----------|-------------|
| `asn_add` | `(string a, string b) → string` | Addition |
| `asn_sub` | `(string a, string b) → string` | Subtraction |
| `asn_mult` | `(string a, string b) → string` | Multiplication |
| `asn_div` | `(string a, string b) → string` | Division |
| `asn_mod` | `(string a, string b) → string` | Modulo |
| `asn_pow` | `(string base, string exp, string mod) → string` | Modular exponentiation |
| `asn_lshift` | `(string a, int n) → string` | Left shift by n bits |
| `asn_rshift` | `(string a, int n) → string` | Right shift by n bits |
| `asn_and` | `(string a, string b) → string` | Bitwise AND |
| `asn_or` | `(string a, string b) → string` | Bitwise OR |
| `asn_xor` | `(string a, string b) → string` | Bitwise XOR |
| `asn_cmp` | `(string a, string b) → int` | Compare: -1, 0, 1 |
| `asn_modinv` | `(string a, string mod) → string` | Modular inverse |

```rust
// Example using num-bigint:
fn kf_asn_add(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let a = BigInt::from_signed_bytes_be(args[0].as_string()?.as_bytes());
    let b = BigInt::from_signed_bytes_be(args[1].as_string()?.as_bytes());
    let result = a + b;
    let bytes = result.to_signed_bytes_be();
    Ok(LpcValue::String(String::from_utf8_lossy(&bytes).into_owned()))
}
```

---

## Category 13: Parse String

**File:** `crates/lpc-vm/src/kfun/parse_string.rs`

### parse_string(string grammar, string input, varargs int max_alt) → mixed*

Context-free grammar parser. This is DGD's most complex kfun.

**Grammar format:**
```
token_rule = /regex/
production : token_rule other_rule ? callback_function
```

**Implementation strategy:**
1. Parse the grammar string into token rules (regexes) and production rules
2. Tokenize the input using the token rules (longest match wins)
3. Parse the token stream using the production rules (bottom-up / Earley parser)
4. On successful rule match, call the named LPC callback function
5. The callback can transform the parse tree node
6. If `max_alt` > 0, keep multiple parses and return best by rule ordering

**DGD reference:** `../dgd/src/parser/` directory contains the full parser
implementation (grammar.cpp, parse.cpp, srp.cpp for SRP tables).

```rust
fn kf_parse_string(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let grammar_str = args[0].as_string()?;
    let input = args[1].as_string()?;
    let max_alt = args.get(2).map(|v| v.as_int()).transpose()?.unwrap_or(1);

    // 1. Parse grammar definition
    let grammar = Grammar::parse(grammar_str)?;

    // 2. Tokenize input using grammar's token rules
    let tokens = grammar.tokenize(input)?;

    // 3. Parse token stream using production rules
    let parse_trees = grammar.parse(&tokens, max_alt as usize)?;

    // 4. For each successful parse, call LPC callback functions
    let mut results = Vec::new();
    for tree in parse_trees {
        let result = evaluate_parse_tree(ctx, &tree)?;
        results.push(result);
    }

    if results.is_empty() {
        Ok(LpcValue::Nil)
    } else if results.len() == 1 {
        Ok(results.into_iter().next().unwrap())
    } else {
        Ok(LpcValue::Array(results))
    }
}
```

**Tests:**
```rust
#[test]
fn parse_string_simple_command() {
    // Grammar that parses "go north" / "go south" / etc.
    let grammar = r#"
        whitespace = /[ \t]+/
        word = /[a-z]+/
        command : word word ? parse_command
    "#;
    let result = eval_with_parse_string(grammar, "go north");
    // Should call parse_command("go", "north") in the LPC object
}

#[test]
fn parse_string_take_item() {
    let grammar = r#"
        whitespace = /[ \t]+/
        word = /[a-z]+/
        article = /the|a|an/
        command : 'take' article ? word ? take_item
    "#;
    let result = eval_with_parse_string(grammar, "take the sword");
    // Should call take_item("take", "the", "sword")
}
```

---

## Category 14: Miscellaneous

**File:** `crates/lpc-vm/src/kfun/misc.rs`

### error(string msg) → void

Throw a runtime error. Equivalent to `throw` in other languages.

```rust
fn kf_error(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let msg = args[0].as_string()?;
    Err(LpcError::RuntimeError(msg.to_string()))
}
```

### call_trace() → mixed**

Returns the current call stack as an array of arrays. Each entry contains:
`({object, function, file, line, is_external})`.

```rust
fn kf_call_trace(ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let trace = ctx.vm.call_stack_trace();
    let entries: Vec<LpcValue> = trace.iter().map(|frame| {
        LpcValue::Array(vec![
            LpcValue::Object(frame.object.clone()),
            LpcValue::String(frame.function.clone()),
            LpcValue::String(frame.file.clone()),
            LpcValue::Int(frame.line as i64),
            LpcValue::Int(if frame.is_external { 1 } else { 0 }),
        ])
    }).collect();
    Ok(LpcValue::Array(entries))
}
```

### status(varargs object obj) → mixed*

Returns resource usage information. Without argument, returns system-wide status.
With object argument, returns that object's status.

**System status array elements:**
```
({
    uptime,           // seconds since boot
    swap_size,        // swap file size
    swap_used,        // swap space used
    sector_size,      // sector size
    free_sectors,     // free sectors
    objects,          // total objects
    call_outs,        // pending call_outs
    ...
})
```

### dump_state(varargs int incremental) → void

Dump persistent state to a snapshot file. If incremental is 1, only dump changes.
Not allowed in atomic functions.

### shutdown(varargs int hotboot) → void

Shut down the driver. If hotboot is 1, perform a hotboot (restart with state
preserved).

### swapout() → void

Force all objects to be swapped to disk.

---

## Category 15: Editor (Optional)

**File:** `crates/lpc-vm/src/kfun/editor.rs`

DGD includes a built-in `ed`-style line editor. This is a legacy feature and
may be low priority for our implementation.

### editor(varargs string command) → string

Execute an editor command. Creates or accesses the editor state for the current
user object.

### query_editor(object obj) → string

Returns the editor status for an object, or nil if no editor is active.

---

## Implementation Order

**Phase 1 — Core (no driver services needed):**
1. `type_ops.rs` — typeof, instanceof
2. `math.rs` — all math functions (pure computation)
3. `string.rs` — strlen, explode, implode, lower_case, upper_case
4. `array.rs` — allocate, allocate_int, allocate_float, sizeof, sort_array
5. `mapping.rs` — map_indices, map_values, map_sizeof, mkmapping
6. `misc.rs` — error, call_trace, time, millitime, ctime, random

**Phase 2 — Object management (needs VM object table):**
7. `object.rs` — this_object, previous_object, clone_object, new_object,
   destruct_object, find_object, object_name, function_object, compile_object

**Phase 3 — Driver service kfuns (needs MOP integration):**
8. `io.rs` — read_file, write_file, remove_file, rename_file, get_dir, make_dir,
   remove_dir
9. `connection.rs` — send_message, users, query_ip_number, query_ip_name, this_user
10. `serialize.rs` — save_object, restore_object
11. `timing.rs` — call_out, remove_call_out

**Phase 4 — Advanced:**
12. `crypto.rs` — crypt, hash_crc16, hash_crc32, hash_string, encrypt, decrypt
13. `asn.rs` — all ASN arithmetic functions
14. `parse_string.rs` — context-free grammar parser
15. `sscanf` (in string.rs) — formatted string scanning
16. `editor.rs` — ed-style line editor (optional/low priority)

---

## Testing Strategy

Each kfun gets:
1. **Unit test** — call the Rust function directly with known inputs
2. **Integration test** — compile and run an LPC program that uses the kfun
3. **Edge case test** — wrong types, nil arguments, boundary values
4. **DGD compatibility test** — verify behavior matches DGD documentation

**Test helper macros:**
```rust
/// Assert a kfun call produces the expected result.
fn assert_kfun_eq(name: &str, args: &[LpcValue], expected: LpcValue) {
    let mut ctx = test_context();
    let registry = KfunRegistry::with_defaults();
    let id = registry.lookup(name).unwrap();
    let result = registry.call(id, &mut ctx, args).unwrap();
    assert_eq!(result, expected, "kfun {} with args {:?}", name, args);
}

/// Assert a kfun call produces an error.
fn assert_kfun_err(name: &str, args: &[LpcValue]) {
    let mut ctx = test_context();
    let registry = KfunRegistry::with_defaults();
    let id = registry.lookup(name).unwrap();
    assert!(registry.call(id, &mut ctx, args).is_err());
}
```
