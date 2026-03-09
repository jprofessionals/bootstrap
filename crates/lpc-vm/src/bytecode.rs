/// Runtime LPC value.
#[derive(Debug, Clone, PartialEq)]
pub enum LpcValue {
    Nil,
    Int(i64),
    Float(f64),
    String(String),
    Array(Vec<LpcValue>),
    Mapping(Vec<(LpcValue, LpcValue)>),
    Object(ObjectRef),
}

/// Reference to a runtime object.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectRef {
    pub id: u64,
    pub path: String,
    pub is_lightweight: bool,
}

impl LpcValue {
    /// Try to extract an integer value.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            LpcValue::Int(v) => Some(*v),
            _ => None,
        }
    }

    /// Try to extract a float value.
    pub fn as_float(&self) -> Option<f64> {
        match self {
            LpcValue::Float(v) => Some(*v),
            _ => None,
        }
    }

    /// Try to extract a string reference.
    pub fn as_string(&self) -> Option<&str> {
        match self {
            LpcValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Try to extract an array reference.
    pub fn as_array(&self) -> Option<&[LpcValue]> {
        match self {
            LpcValue::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Try to extract a mapping reference.
    pub fn as_mapping(&self) -> Option<&[(LpcValue, LpcValue)]> {
        match self {
            LpcValue::Mapping(m) => Some(m),
            _ => None,
        }
    }

    /// Try to extract an object reference.
    pub fn as_object(&self) -> Option<&ObjectRef> {
        match self {
            LpcValue::Object(o) => Some(o),
            _ => None,
        }
    }

    /// Return the LPC type name for this value.
    pub fn type_name(&self) -> &'static str {
        match self {
            LpcValue::Nil => "nil",
            LpcValue::Int(_) => "int",
            LpcValue::Float(_) => "float",
            LpcValue::String(_) => "string",
            LpcValue::Array(_) => "array",
            LpcValue::Mapping(_) => "mapping",
            LpcValue::Object(_) => "object",
        }
    }

    /// Check whether this value is truthy in LPC semantics.
    pub fn is_truthy(&self) -> bool {
        match self {
            LpcValue::Nil => false,
            LpcValue::Int(v) => *v != 0,
            LpcValue::Float(v) => *v != 0.0,
            LpcValue::String(s) => !s.is_empty(),
            LpcValue::Array(a) => !a.is_empty(),
            LpcValue::Mapping(m) => !m.is_empty(),
            LpcValue::Object(_) => true,
        }
    }
}

/// Bytecode instruction.
#[derive(Debug, Clone)]
pub enum OpCode {
    // Constants
    PushNil,
    PushInt(i64),
    PushFloat(f64),
    PushString(String),

    // Stack
    Pop,
    Dup,

    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Neg,

    // Comparison
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,

    // Logical
    Not,
    And,
    Or,

    // Bitwise
    BitAnd,
    BitOr,
    BitXor,
    BitNot,
    Shl,
    Shr,

    // String
    StrConcat,

    // Variables
    GetLocal(u16),
    SetLocal(u16),
    GetGlobal(u16),
    SetGlobal(u16),

    // Compound assignment
    AddAssignLocal(u16),
    SubAssignLocal(u16),
    MulAssignLocal(u16),
    DivAssignLocal(u16),
    ModAssignLocal(u16),
    AndAssignLocal(u16),
    OrAssignLocal(u16),
    XorAssignLocal(u16),
    ShlAssignLocal(u16),
    ShrAssignLocal(u16),
    AddAssignGlobal(u16),
    SubAssignGlobal(u16),
    MulAssignGlobal(u16),
    DivAssignGlobal(u16),
    ModAssignGlobal(u16),
    AndAssignGlobal(u16),
    OrAssignGlobal(u16),
    XorAssignGlobal(u16),
    ShlAssignGlobal(u16),
    ShrAssignGlobal(u16),

    // Control flow
    Jump(i32),
    JumpIfFalse(i32),
    JumpIfTrue(i32),

    // Functions
    Call {
        func_idx: u16,
        arg_count: u8,
    },
    CallOther {
        arg_count: u8,
    },
    CallParent {
        func_name: String,
        arg_count: u8,
    },
    CallKfun {
        kfun_id: u16,
        arg_count: u8,
    },
    Return,
    ReturnNil,

    // Objects
    CloneObject,
    NewObject,
    DestructObject,
    ThisObject,

    // Collections
    MakeArray(u16),
    MakeMapping(u16),
    Index,
    IndexAssign,
    RangeIndex,
    Sizeof,

    // Type
    TypeOf,
    CastType(u8),

    // Increment/Decrement
    PreIncLocal(u16),
    PreDecLocal(u16),
    PostIncLocal(u16),
    PostDecLocal(u16),

    // Tick control
    CheckTicks(u32),
}

/// A compiled function.
#[derive(Debug, Clone)]
pub struct CompiledFunction {
    pub name: String,
    pub arity: u16,
    pub varargs: bool,
    pub local_count: u16,
    pub code: Vec<OpCode>,
    pub modifiers: Vec<u8>,
}

/// A compiled LPC program (one .c file).
#[derive(Debug, Clone)]
pub struct CompiledProgram {
    pub path: String,
    pub version: u64,
    pub inherits: Vec<String>,
    pub functions: Vec<CompiledFunction>,
    pub global_count: u16,
    pub global_names: Vec<String>,
}
