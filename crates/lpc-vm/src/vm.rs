use crate::bytecode::{CompiledProgram, LpcValue, ObjectRef, OpCode};
use crate::compiler::Compiler;
use crate::kfun::{KfunContext, KfunRegistry, LpcError};
use crate::object::{ObjectError, ObjectTable};
use crate::parser::Parser;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Runtime errors raised during VM execution.
#[derive(Debug, Clone, thiserror::Error)]
pub enum VmError {
    #[error("execution ticks exhausted")]
    TicksExhausted,

    #[error("stack overflow")]
    StackOverflow,

    #[error("type error: {0}")]
    TypeError(String),

    #[error("division by zero")]
    DivisionByZero,

    #[error("undefined function `{0}`")]
    UndefinedFunction(String),

    #[error("undefined variable index {0}")]
    UndefinedVariable(u16),

    #[error("object destroyed")]
    ObjectDestroyed,

    #[error("index out of bounds: {index} (size {size})")]
    IndexOutOfBounds { index: i64, size: usize },

    #[error("atomic violation: {0}")]
    AtomicViolation(String),

    #[error("runtime error: {0}")]
    RuntimeError(String),

    #[error("compile error: {0}")]
    CompileError(String),
}

impl From<ObjectError> for VmError {
    fn from(e: ObjectError) -> Self {
        match e {
            ObjectError::NotFound(p) => VmError::RuntimeError(format!("object not found: {p}")),
            ObjectError::NotMaster(p) => VmError::RuntimeError(format!("not a master object: {p}")),
            ObjectError::AlreadyDestroyed(_) => VmError::ObjectDestroyed,
            ObjectError::InvalidClone(id) => {
                VmError::RuntimeError(format!("invalid clone id: {id}"))
            }
            ObjectError::InheritanceCycle(p) => {
                VmError::RuntimeError(format!("inheritance cycle: {p}"))
            }
        }
    }
}

impl From<LpcError> for VmError {
    fn from(e: LpcError) -> Self {
        match e {
            LpcError::TypeError {
                expected,
                got,
                arg_pos,
            } => VmError::TypeError(format!(
                "expected {expected}, got {got} at argument {arg_pos}"
            )),
            LpcError::ValueError(msg) => VmError::RuntimeError(msg),
            LpcError::RuntimeError(msg) => VmError::RuntimeError(msg),
            LpcError::AtomicViolation(msg) => VmError::AtomicViolation(msg),
        }
    }
}

// ---------------------------------------------------------------------------
// Frame info (for stack traces)
// ---------------------------------------------------------------------------

/// Human-readable call-stack entry.
#[derive(Debug, Clone)]
pub struct FrameInfo {
    pub program_path: String,
    pub function_name: String,
    pub ip: usize,
}

// ---------------------------------------------------------------------------
// Scheduler (stub)
// ---------------------------------------------------------------------------

/// Placeholder scheduler for deferred call-outs.
pub struct Scheduler {
    // Will be expanded later.
}

impl Scheduler {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Execution state (private)
// ---------------------------------------------------------------------------

struct CallFrame {
    function_idx: usize,
    program_path: String,
    ip: usize,
    #[allow(dead_code)]
    base_pointer: usize,
    locals: Vec<LpcValue>,
    this_object: ObjectRef,
    previous_object: Option<ObjectRef>,
}

struct ExecutionState {
    stack: Vec<LpcValue>,
    call_stack: Vec<CallFrame>,
    tick_counter: u64,
    #[allow(dead_code)]
    atomic_depth: u32,
}

// ---------------------------------------------------------------------------
// The VM
// ---------------------------------------------------------------------------

/// The LPC bytecode virtual machine.
pub struct Vm {
    pub object_table: ObjectTable,
    pub kfun_registry: KfunRegistry,
    pub scheduler: Scheduler,
    pub rng: rand::rngs::StdRng,
    tick_limit: u64,
    stack_limit: usize,
}

impl Vm {
    /// Create a new VM with default limits.
    pub fn new() -> Self {
        use rand::SeedableRng;
        let mut kfuns = KfunRegistry::new();
        kfuns.register_defaults();
        Self {
            object_table: ObjectTable::new(),
            kfun_registry: kfuns,
            scheduler: Scheduler::new(),
            rng: rand::rngs::StdRng::from_os_rng(),
            tick_limit: 1_000_000,
            stack_limit: 4096,
        }
    }

    /// Load a pre-compiled program into the VM as a master object.
    pub fn load_program(&mut self, program: CompiledProgram) -> ObjectRef {
        self.object_table.register_master(program)
    }

    /// Compile LPC source code, load it, and return the master object reference.
    pub fn compile_and_load(&mut self, path: &str, source: &str) -> Result<ObjectRef, VmError> {
        use crate::lexer::scanner::Scanner;

        let mut scanner = Scanner::new(source);
        let tokens = scanner
            .scan_all()
            .map_err(|e| VmError::CompileError(format!("{e}")))?;
        let mut parser = Parser::new(tokens);
        let program_ast = parser
            .parse_program()
            .map_err(|e| VmError::CompileError(format!("{e}")))?;
        let mut compiler = Compiler::new();
        let mut compiled = compiler
            .compile(&program_ast)
            .map_err(|e| VmError::CompileError(format!("{e}")))?;
        compiled.path = path.to_string();
        let oref = self.object_table.register_master(compiled);
        Ok(oref)
    }

    /// Execute a named function on an object.
    pub fn call_function(
        &mut self,
        object: &ObjectRef,
        function: &str,
        args: &[LpcValue],
    ) -> Result<LpcValue, VmError> {
        // Resolve function through the inheritance chain.
        let (program_path, func_idx) =
            self.object_table
                .resolve_function(&object.path, function)
                .ok_or_else(|| VmError::UndefinedFunction(function.to_string()))?;

        let program = self
            .object_table
            .get_master(&program_path)
            .ok_or_else(|| VmError::RuntimeError(format!("no program for {}", program_path)))?
            .program
            .clone();

        let func = &program.functions[func_idx];

        // Build locals: first slots are the arguments, rest are Nil.
        let mut locals = Vec::with_capacity(func.local_count as usize);
        for i in 0..func.local_count as usize {
            if i < args.len() {
                locals.push(args[i].clone());
            } else {
                locals.push(LpcValue::Nil);
            }
        }

        let frame = CallFrame {
            function_idx: func_idx,
            program_path,
            ip: 0,
            base_pointer: 0,
            locals,
            this_object: object.clone(),
            previous_object: None,
        };

        let mut state = ExecutionState {
            stack: Vec::with_capacity(256),
            call_stack: vec![frame],
            tick_counter: self.tick_limit,
            atomic_depth: 0,
        };

        self.execute(&mut state)
    }

    /// Check whether an object has a named function.
    pub fn has_function(&self, object: &ObjectRef, name: &str) -> bool {
        self.object_table
            .resolve_function(&object.path, name)
            .is_some()
    }

    /// Return a human-readable call stack trace from the current execution.
    pub fn call_stack_trace(&self) -> Vec<FrameInfo> {
        // Only meaningful during execution; returns empty otherwise.
        Vec::new()
    }

    /// Whether the VM is currently inside an atomic block.
    pub fn is_atomic(&self) -> bool {
        false
    }

    /// Get the caller at depth `depth` in the call stack.
    pub fn get_caller(&self, _depth: usize) -> Option<&ObjectRef> {
        None
    }

    // -----------------------------------------------------------------------
    // Private: the execution loop
    // -----------------------------------------------------------------------

    fn get_program_for_path(&self, path: &str) -> Result<CompiledProgram, VmError> {
        self.object_table
            .get_master(path)
            .map(|m| m.program.clone())
            .ok_or_else(|| VmError::RuntimeError(format!("no program for path `{path}`")))
    }

    fn execute(&mut self, state: &mut ExecutionState) -> Result<LpcValue, VmError> {
        loop {
            // ---- fetch ----
            let frame = state.call_stack.last().unwrap();
            let program = self.get_program_for_path(&frame.program_path)?;
            let func = &program.functions[frame.function_idx];

            if frame.ip >= func.code.len() {
                // Implicit return nil at end of function.
                state.call_stack.pop();
                if state.call_stack.is_empty() {
                    return Ok(LpcValue::Nil);
                }
                state.stack.push(LpcValue::Nil);
                continue;
            }

            // Tick accounting.
            state.tick_counter = state
                .tick_counter
                .checked_sub(1)
                .ok_or(VmError::TicksExhausted)?;

            if state.stack.len() > self.stack_limit {
                return Err(VmError::StackOverflow);
            }

            let op = func.code[frame.ip].clone();
            state.call_stack.last_mut().unwrap().ip += 1;

            match op {
                // -----------------------------------------------------------
                // Constants
                // -----------------------------------------------------------
                OpCode::PushNil => state.stack.push(LpcValue::Nil),
                OpCode::PushInt(n) => state.stack.push(LpcValue::Int(n)),
                OpCode::PushFloat(f) => state.stack.push(LpcValue::Float(f)),
                OpCode::PushString(s) => state.stack.push(LpcValue::String(s)),

                // -----------------------------------------------------------
                // Stack manipulation
                // -----------------------------------------------------------
                OpCode::Pop => {
                    state.stack.pop();
                }
                OpCode::Dup => {
                    let v = state.stack.last().cloned().unwrap_or(LpcValue::Nil);
                    state.stack.push(v);
                }

                // -----------------------------------------------------------
                // Arithmetic
                // -----------------------------------------------------------
                OpCode::Add => {
                    let b = pop(&mut state.stack)?;
                    let a = pop(&mut state.stack)?;
                    state.stack.push(arith_add(a, b)?);
                }
                OpCode::Sub => {
                    let b = pop(&mut state.stack)?;
                    let a = pop(&mut state.stack)?;
                    state.stack.push(arith_sub(a, b)?);
                }
                OpCode::Mul => {
                    let b = pop(&mut state.stack)?;
                    let a = pop(&mut state.stack)?;
                    state.stack.push(arith_mul(a, b)?);
                }
                OpCode::Div => {
                    let b = pop(&mut state.stack)?;
                    let a = pop(&mut state.stack)?;
                    state.stack.push(arith_div(a, b)?);
                }
                OpCode::Mod => {
                    let b = pop(&mut state.stack)?;
                    let a = pop(&mut state.stack)?;
                    state.stack.push(arith_mod(a, b)?);
                }
                OpCode::Neg => {
                    let v = pop(&mut state.stack)?;
                    state.stack.push(arith_neg(v)?);
                }

                // -----------------------------------------------------------
                // Comparison
                // -----------------------------------------------------------
                OpCode::Eq => {
                    let b = pop(&mut state.stack)?;
                    let a = pop(&mut state.stack)?;
                    state
                        .stack
                        .push(LpcValue::Int(if val_eq(&a, &b) { 1 } else { 0 }));
                }
                OpCode::Ne => {
                    let b = pop(&mut state.stack)?;
                    let a = pop(&mut state.stack)?;
                    state
                        .stack
                        .push(LpcValue::Int(if val_eq(&a, &b) { 0 } else { 1 }));
                }
                OpCode::Lt => {
                    let b = pop(&mut state.stack)?;
                    let a = pop(&mut state.stack)?;
                    state
                        .stack
                        .push(LpcValue::Int(if val_cmp(&a, &b)? < 0 { 1 } else { 0 }));
                }
                OpCode::Gt => {
                    let b = pop(&mut state.stack)?;
                    let a = pop(&mut state.stack)?;
                    state
                        .stack
                        .push(LpcValue::Int(if val_cmp(&a, &b)? > 0 { 1 } else { 0 }));
                }
                OpCode::Le => {
                    let b = pop(&mut state.stack)?;
                    let a = pop(&mut state.stack)?;
                    state
                        .stack
                        .push(LpcValue::Int(if val_cmp(&a, &b)? <= 0 { 1 } else { 0 }));
                }
                OpCode::Ge => {
                    let b = pop(&mut state.stack)?;
                    let a = pop(&mut state.stack)?;
                    state
                        .stack
                        .push(LpcValue::Int(if val_cmp(&a, &b)? >= 0 { 1 } else { 0 }));
                }

                // -----------------------------------------------------------
                // Logical
                // -----------------------------------------------------------
                OpCode::Not => {
                    let v = pop(&mut state.stack)?;
                    state
                        .stack
                        .push(LpcValue::Int(if v.is_truthy() { 0 } else { 1 }));
                }
                OpCode::And => {
                    let b = pop(&mut state.stack)?;
                    let a = pop(&mut state.stack)?;
                    state.stack.push(if !a.is_truthy() { a } else { b });
                }
                OpCode::Or => {
                    let b = pop(&mut state.stack)?;
                    let a = pop(&mut state.stack)?;
                    state.stack.push(if a.is_truthy() { a } else { b });
                }

                // -----------------------------------------------------------
                // Bitwise
                // -----------------------------------------------------------
                OpCode::BitAnd => {
                    let b = pop(&mut state.stack)?;
                    let a = pop(&mut state.stack)?;
                    state.stack.push(bitwise_and(a, b)?);
                }
                OpCode::BitOr => {
                    let b = pop(&mut state.stack)?;
                    let a = pop(&mut state.stack)?;
                    state.stack.push(bitwise_or(a, b)?);
                }
                OpCode::BitXor => {
                    let b = pop(&mut state.stack)?;
                    let a = pop(&mut state.stack)?;
                    match (&a, &b) {
                        (LpcValue::Int(x), LpcValue::Int(y)) => {
                            state.stack.push(LpcValue::Int(x ^ y));
                        }
                        _ => {
                            return Err(VmError::TypeError(format!(
                                "bitwise xor requires int, got {} ^ {}",
                                a.type_name(),
                                b.type_name()
                            )));
                        }
                    }
                }
                OpCode::BitNot => {
                    let v = pop(&mut state.stack)?;
                    match v {
                        LpcValue::Int(n) => state.stack.push(LpcValue::Int(!n)),
                        _ => {
                            return Err(VmError::TypeError(format!(
                                "bitwise not requires int, got {}",
                                v.type_name()
                            )));
                        }
                    }
                }
                OpCode::Shl => {
                    let b = pop(&mut state.stack)?;
                    let a = pop(&mut state.stack)?;
                    match (&a, &b) {
                        (LpcValue::Int(x), LpcValue::Int(y)) => {
                            state.stack.push(LpcValue::Int(x.wrapping_shl(*y as u32)));
                        }
                        _ => {
                            return Err(VmError::TypeError(format!(
                                "shift left requires int, got {} << {}",
                                a.type_name(),
                                b.type_name()
                            )));
                        }
                    }
                }
                OpCode::Shr => {
                    let b = pop(&mut state.stack)?;
                    let a = pop(&mut state.stack)?;
                    match (&a, &b) {
                        (LpcValue::Int(x), LpcValue::Int(y)) => {
                            state.stack.push(LpcValue::Int(x.wrapping_shr(*y as u32)));
                        }
                        _ => {
                            return Err(VmError::TypeError(format!(
                                "shift right requires int, got {} >> {}",
                                a.type_name(),
                                b.type_name()
                            )));
                        }
                    }
                }

                // -----------------------------------------------------------
                // String
                // -----------------------------------------------------------
                OpCode::StrConcat => {
                    let b = pop(&mut state.stack)?;
                    let a = pop(&mut state.stack)?;
                    match (a, b) {
                        (LpcValue::String(mut sa), LpcValue::String(sb)) => {
                            sa.push_str(&sb);
                            state.stack.push(LpcValue::String(sa));
                        }
                        (a, b) => {
                            return Err(VmError::TypeError(format!(
                                "string concat requires string, got {} + {}",
                                a.type_name(),
                                b.type_name()
                            )));
                        }
                    }
                }

                // -----------------------------------------------------------
                // Variables
                // -----------------------------------------------------------
                OpCode::GetLocal(idx) => {
                    let frame = state.call_stack.last().unwrap();
                    let val = frame
                        .locals
                        .get(idx as usize)
                        .cloned()
                        .unwrap_or(LpcValue::Nil);
                    state.stack.push(val);
                }
                OpCode::SetLocal(idx) => {
                    let val = pop(&mut state.stack)?;
                    let frame = state.call_stack.last_mut().unwrap();
                    let idx = idx as usize;
                    if idx >= frame.locals.len() {
                        frame.locals.resize(idx + 1, LpcValue::Nil);
                    }
                    frame.locals[idx] = val.clone();
                    state.stack.push(val);
                }
                OpCode::GetGlobal(idx) => {
                    let this_obj = &state.call_stack.last().unwrap().this_object;
                    let val = self
                        .object_table
                        .get_global(this_obj, idx)
                        .cloned()
                        .unwrap_or(LpcValue::Nil);
                    state.stack.push(val);
                }
                OpCode::SetGlobal(idx) => {
                    let val = pop(&mut state.stack)?;
                    let this_obj = state.call_stack.last().unwrap().this_object.clone();
                    let _ = self.object_table.set_global(&this_obj, idx, val.clone());
                    state.stack.push(val);
                }

                // -----------------------------------------------------------
                // Compound assignment (local)
                // -----------------------------------------------------------
                OpCode::AddAssignLocal(idx) => {
                    compound_assign_local(state, idx, arith_add)?;
                }
                OpCode::SubAssignLocal(idx) => {
                    compound_assign_local(state, idx, arith_sub)?;
                }
                OpCode::MulAssignLocal(idx) => {
                    compound_assign_local(state, idx, arith_mul)?;
                }
                OpCode::DivAssignLocal(idx) => {
                    compound_assign_local(state, idx, arith_div)?;
                }
                OpCode::ModAssignLocal(idx) => {
                    compound_assign_local(state, idx, arith_mod)?;
                }
                OpCode::AndAssignLocal(idx) => {
                    compound_assign_local(state, idx, bitwise_and)?;
                }
                OpCode::OrAssignLocal(idx) => {
                    compound_assign_local(state, idx, bitwise_or)?;
                }
                OpCode::XorAssignLocal(idx) => {
                    compound_assign_local(state, idx, |a, b| match (a, b) {
                        (LpcValue::Int(x), LpcValue::Int(y)) => Ok(LpcValue::Int(x ^ y)),
                        (a, b) => Err(VmError::TypeError(format!(
                            "^= requires int, got {} and {}",
                            a.type_name(),
                            b.type_name()
                        ))),
                    })?;
                }
                OpCode::ShlAssignLocal(idx) => {
                    compound_assign_local(state, idx, |a, b| match (a, b) {
                        (LpcValue::Int(x), LpcValue::Int(y)) => {
                            Ok(LpcValue::Int(x.wrapping_shl(y as u32)))
                        }
                        (a, b) => Err(VmError::TypeError(format!(
                            "<<= requires int, got {} and {}",
                            a.type_name(),
                            b.type_name()
                        ))),
                    })?;
                }
                OpCode::ShrAssignLocal(idx) => {
                    compound_assign_local(state, idx, |a, b| match (a, b) {
                        (LpcValue::Int(x), LpcValue::Int(y)) => {
                            Ok(LpcValue::Int(x.wrapping_shr(y as u32)))
                        }
                        (a, b) => Err(VmError::TypeError(format!(
                            ">>= requires int, got {} and {}",
                            a.type_name(),
                            b.type_name()
                        ))),
                    })?;
                }

                // -----------------------------------------------------------
                // Compound assignment (global)
                // -----------------------------------------------------------
                OpCode::AddAssignGlobal(idx) => {
                    compound_assign_global(state, &mut self.object_table, idx, arith_add)?;
                }
                OpCode::SubAssignGlobal(idx) => {
                    compound_assign_global(state, &mut self.object_table, idx, arith_sub)?;
                }
                OpCode::MulAssignGlobal(idx) => {
                    compound_assign_global(state, &mut self.object_table, idx, arith_mul)?;
                }
                OpCode::DivAssignGlobal(idx) => {
                    compound_assign_global(state, &mut self.object_table, idx, arith_div)?;
                }
                OpCode::ModAssignGlobal(idx) => {
                    compound_assign_global(state, &mut self.object_table, idx, arith_mod)?;
                }
                OpCode::AndAssignGlobal(idx) => {
                    compound_assign_global(state, &mut self.object_table, idx, bitwise_and)?;
                }
                OpCode::OrAssignGlobal(idx) => {
                    compound_assign_global(state, &mut self.object_table, idx, bitwise_or)?;
                }
                OpCode::XorAssignGlobal(idx) => {
                    compound_assign_global(state, &mut self.object_table, idx, |a, b| {
                        match (a, b) {
                            (LpcValue::Int(x), LpcValue::Int(y)) => Ok(LpcValue::Int(x ^ y)),
                            (a, b) => Err(VmError::TypeError(format!(
                                "^= requires int, got {} and {}",
                                a.type_name(),
                                b.type_name()
                            ))),
                        }
                    })?;
                }
                OpCode::ShlAssignGlobal(idx) => {
                    compound_assign_global(state, &mut self.object_table, idx, |a, b| {
                        match (a, b) {
                            (LpcValue::Int(x), LpcValue::Int(y)) => {
                                Ok(LpcValue::Int(x.wrapping_shl(y as u32)))
                            }
                            (a, b) => Err(VmError::TypeError(format!(
                                "<<= requires int, got {} and {}",
                                a.type_name(),
                                b.type_name()
                            ))),
                        }
                    })?;
                }
                OpCode::ShrAssignGlobal(idx) => {
                    compound_assign_global(state, &mut self.object_table, idx, |a, b| {
                        match (a, b) {
                            (LpcValue::Int(x), LpcValue::Int(y)) => {
                                Ok(LpcValue::Int(x.wrapping_shr(y as u32)))
                            }
                            (a, b) => Err(VmError::TypeError(format!(
                                ">>= requires int, got {} and {}",
                                a.type_name(),
                                b.type_name()
                            ))),
                        }
                    })?;
                }

                // -----------------------------------------------------------
                // Control flow
                // -----------------------------------------------------------
                OpCode::Jump(offset) => {
                    let frame = state.call_stack.last_mut().unwrap();
                    frame.ip = (frame.ip as i64 + offset as i64 - 1) as usize;
                }
                OpCode::JumpIfFalse(offset) => {
                    let v = pop(&mut state.stack)?;
                    if !v.is_truthy() {
                        let frame = state.call_stack.last_mut().unwrap();
                        frame.ip = (frame.ip as i64 + offset as i64 - 1) as usize;
                    }
                }
                OpCode::JumpIfTrue(offset) => {
                    let v = pop(&mut state.stack)?;
                    if v.is_truthy() {
                        let frame = state.call_stack.last_mut().unwrap();
                        frame.ip = (frame.ip as i64 + offset as i64 - 1) as usize;
                    }
                }

                // -----------------------------------------------------------
                // Function calls
                // -----------------------------------------------------------
                OpCode::Call {
                    func_idx,
                    arg_count,
                } => {
                    let arg_count = arg_count as usize;
                    let mut args = Vec::with_capacity(arg_count);
                    for _ in 0..arg_count {
                        args.push(pop(&mut state.stack)?);
                    }
                    args.reverse();

                    let caller_frame = state.call_stack.last().unwrap();
                    let prog_path = caller_frame.program_path.clone();
                    let caller_obj = caller_frame.this_object.clone();

                    let target_program = self.get_program_for_path(&prog_path)?;
                    let target_func = target_program
                        .functions
                        .get(func_idx as usize)
                        .ok_or_else(|| VmError::UndefinedFunction(format!("func#{func_idx}")))?;

                    let mut locals = Vec::with_capacity(target_func.local_count as usize);
                    for i in 0..target_func.local_count as usize {
                        if i < args.len() {
                            locals.push(args[i].clone());
                        } else {
                            locals.push(LpcValue::Nil);
                        }
                    }

                    let new_frame = CallFrame {
                        function_idx: func_idx as usize,
                        program_path: prog_path,
                        ip: 0,
                        base_pointer: state.stack.len(),
                        locals,
                        this_object: caller_obj.clone(),
                        previous_object: Some(caller_obj),
                    };
                    state.call_stack.push(new_frame);
                }

                OpCode::CallOther { arg_count } => {
                    // Stack layout: [target_obj, func_name, arg0, arg1, ...]
                    // Pop args first (they are on top).
                    let arg_count = arg_count as usize;
                    let mut args = Vec::with_capacity(arg_count);
                    for _ in 0..arg_count {
                        args.push(pop(&mut state.stack)?);
                    }
                    args.reverse();

                    let func_name_val = pop(&mut state.stack)?;
                    let target_val = pop(&mut state.stack)?;

                    let func_name = match &func_name_val {
                        LpcValue::String(s) => s.clone(),
                        _ => {
                            return Err(VmError::TypeError(
                                "call_other: function name must be a string".into(),
                            ));
                        }
                    };

                    let target_ref = match &target_val {
                        LpcValue::Object(o) => o.clone(),
                        LpcValue::String(path) => {
                            self.object_table.find_object(path).ok_or_else(|| {
                                VmError::RuntimeError(format!("object not found: {path}"))
                            })?
                        }
                        _ => {
                            return Err(VmError::TypeError(
                                "call_other: target must be object or string".into(),
                            ));
                        }
                    };

                    let (resolved_path, func_idx) = self
                        .object_table
                        .resolve_function(&target_ref.path, &func_name)
                        .ok_or_else(|| VmError::UndefinedFunction(func_name.clone()))?;

                    let target_program = self.get_program_for_path(&resolved_path)?;
                    let target_func = &target_program.functions[func_idx];

                    let mut locals = Vec::with_capacity(target_func.local_count as usize);
                    for i in 0..target_func.local_count as usize {
                        if i < args.len() {
                            locals.push(args[i].clone());
                        } else {
                            locals.push(LpcValue::Nil);
                        }
                    }

                    let caller_obj = state.call_stack.last().unwrap().this_object.clone();

                    let new_frame = CallFrame {
                        function_idx: func_idx,
                        program_path: resolved_path,
                        ip: 0,
                        base_pointer: state.stack.len(),
                        locals,
                        this_object: target_ref,
                        previous_object: Some(caller_obj),
                    };
                    state.call_stack.push(new_frame);
                }

                OpCode::CallParent {
                    func_name,
                    arg_count,
                } => {
                    let arg_count = arg_count as usize;
                    let mut args = Vec::with_capacity(arg_count);
                    for _ in 0..arg_count {
                        args.push(pop(&mut state.stack)?);
                    }
                    args.reverse();

                    let caller_frame = state.call_stack.last().unwrap();
                    let prog_path = caller_frame.program_path.clone();
                    let caller_obj = caller_frame.this_object.clone();

                    // Look up the parent program through the inherits chain.
                    let current_program = self.get_program_for_path(&prog_path)?;
                    let mut found = None;
                    for inherit_path in &current_program.inherits {
                        if let Some((resolved_path, idx)) =
                            self.object_table.resolve_function(inherit_path, &func_name)
                        {
                            found = Some((resolved_path, idx));
                            break;
                        }
                    }

                    let (parent_path, func_idx) = found.ok_or_else(|| {
                        VmError::UndefinedFunction(format!("parent::{func_name}"))
                    })?;

                    let parent_program = self.get_program_for_path(&parent_path)?;
                    let parent_func = &parent_program.functions[func_idx];
                    let mut locals = Vec::with_capacity(parent_func.local_count as usize);
                    for i in 0..parent_func.local_count as usize {
                        if i < args.len() {
                            locals.push(args[i].clone());
                        } else {
                            locals.push(LpcValue::Nil);
                        }
                    }

                    let new_frame = CallFrame {
                        function_idx: func_idx,
                        program_path: parent_path,
                        ip: 0,
                        base_pointer: state.stack.len(),
                        locals,
                        this_object: caller_obj.clone(),
                        previous_object: Some(caller_obj),
                    };
                    state.call_stack.push(new_frame);
                }

                OpCode::CallKfun { kfun_id, arg_count } => {
                    let arg_count = arg_count as usize;
                    let mut args = Vec::with_capacity(arg_count);
                    for _ in 0..arg_count {
                        args.push(pop(&mut state.stack)?);
                    }
                    args.reverse();

                    let this_obj = state.call_stack.last().unwrap().this_object.clone();
                    let prev_obj = state.call_stack.last().unwrap().previous_object.clone();

                    let mut ctx = KfunContext {
                        this_object: &this_obj,
                        previous_object: prev_obj.as_ref(),
                        tick_counter: &mut state.tick_counter,
                    };

                    let result = self.kfun_registry.call(kfun_id, &mut ctx, &args)?;
                    state.stack.push(result);
                }

                OpCode::Return => {
                    let value = state.stack.pop().unwrap_or(LpcValue::Nil);
                    state.call_stack.pop();
                    if state.call_stack.is_empty() {
                        return Ok(value);
                    }
                    state.stack.push(value);
                }

                OpCode::ReturnNil => {
                    state.call_stack.pop();
                    if state.call_stack.is_empty() {
                        return Ok(LpcValue::Nil);
                    }
                    state.stack.push(LpcValue::Nil);
                }

                // -----------------------------------------------------------
                // Objects
                // -----------------------------------------------------------
                OpCode::CloneObject => {
                    let src = pop(&mut state.stack)?;
                    match src {
                        LpcValue::Object(oref) => {
                            let new_ref = self.object_table.clone_object(&oref.path)?;
                            state.stack.push(LpcValue::Object(new_ref));
                        }
                        LpcValue::String(path) => {
                            let new_ref = self.object_table.clone_object(&path)?;
                            state.stack.push(LpcValue::Object(new_ref));
                        }
                        other => {
                            return Err(VmError::TypeError(format!(
                                "clone_object requires object or string, got {}",
                                other.type_name()
                            )));
                        }
                    }
                }

                OpCode::NewObject => {
                    let path_val = pop(&mut state.stack)?;
                    match path_val {
                        LpcValue::String(path) => {
                            let new_ref = self.object_table.new_lightweight(&path)?;
                            state.stack.push(LpcValue::Object(new_ref));
                        }
                        _ => {
                            return Err(VmError::TypeError(
                                "new_object requires string path".into(),
                            ));
                        }
                    }
                }

                OpCode::DestructObject => {
                    let val = pop(&mut state.stack)?;
                    match val {
                        LpcValue::Object(oref) => {
                            self.object_table.destruct(&oref)?;
                        }
                        _ => {
                            return Err(VmError::TypeError("destruct requires object".into()));
                        }
                    }
                }

                OpCode::ThisObject => {
                    let oref = state.call_stack.last().unwrap().this_object.clone();
                    state.stack.push(LpcValue::Object(oref));
                }

                // -----------------------------------------------------------
                // Collections
                // -----------------------------------------------------------
                OpCode::MakeArray(count) => {
                    let count = count as usize;
                    let start = state.stack.len().saturating_sub(count);
                    let elements: Vec<LpcValue> = state.stack.drain(start..).collect();
                    state.stack.push(LpcValue::Array(elements));
                }

                OpCode::MakeMapping(count) => {
                    let count = count as usize;
                    // `count` is the number of key-value pairs, so 2*count items on stack.
                    let item_count = count * 2;
                    let start = state.stack.len().saturating_sub(item_count);
                    let items: Vec<LpcValue> = state.stack.drain(start..).collect();
                    let mut pairs = Vec::with_capacity(count);
                    for chunk in items.chunks_exact(2) {
                        pairs.push((chunk[0].clone(), chunk[1].clone()));
                    }
                    state.stack.push(LpcValue::Mapping(pairs));
                }

                OpCode::Index => {
                    let index = pop(&mut state.stack)?;
                    let collection = pop(&mut state.stack)?;
                    state.stack.push(index_value(&collection, &index)?);
                }

                OpCode::IndexAssign => {
                    let value = pop(&mut state.stack)?;
                    let index = pop(&mut state.stack)?;
                    let mut collection = pop(&mut state.stack)?;
                    index_assign(&mut collection, &index, value)?;
                    state.stack.push(collection);
                }

                OpCode::RangeIndex => {
                    let end = pop(&mut state.stack)?;
                    let start = pop(&mut state.stack)?;
                    let collection = pop(&mut state.stack)?;
                    state.stack.push(range_index(&collection, &start, &end)?);
                }

                OpCode::Sizeof => {
                    let val = pop(&mut state.stack)?;
                    let size = match &val {
                        LpcValue::String(s) => s.len() as i64,
                        LpcValue::Array(a) => a.len() as i64,
                        LpcValue::Mapping(m) => m.len() as i64,
                        LpcValue::Nil => 0,
                        _ => {
                            return Err(VmError::TypeError(format!(
                                "sizeof requires string/array/mapping, got {}",
                                val.type_name()
                            )));
                        }
                    };
                    state.stack.push(LpcValue::Int(size));
                }

                // -----------------------------------------------------------
                // Type operations
                // -----------------------------------------------------------
                OpCode::TypeOf => {
                    let val = pop(&mut state.stack)?;
                    state
                        .stack
                        .push(LpcValue::String(val.type_name().to_string()));
                }

                OpCode::CastType(type_id) => {
                    let val = pop(&mut state.stack)?;
                    state.stack.push(cast_value(val, type_id)?);
                }

                // -----------------------------------------------------------
                // Increment / Decrement
                // -----------------------------------------------------------
                OpCode::PreIncLocal(idx) => {
                    let frame = state.call_stack.last_mut().unwrap();
                    let idx = idx as usize;
                    if idx >= frame.locals.len() {
                        frame.locals.resize(idx + 1, LpcValue::Nil);
                    }
                    let new_val = match &frame.locals[idx] {
                        LpcValue::Int(n) => LpcValue::Int(n + 1),
                        LpcValue::Float(f) => LpcValue::Float(f + 1.0),
                        other => {
                            return Err(VmError::TypeError(format!(
                                "cannot increment {}",
                                other.type_name()
                            )));
                        }
                    };
                    frame.locals[idx] = new_val.clone();
                    state.stack.push(new_val);
                }

                OpCode::PreDecLocal(idx) => {
                    let frame = state.call_stack.last_mut().unwrap();
                    let idx = idx as usize;
                    if idx >= frame.locals.len() {
                        frame.locals.resize(idx + 1, LpcValue::Nil);
                    }
                    let new_val = match &frame.locals[idx] {
                        LpcValue::Int(n) => LpcValue::Int(n - 1),
                        LpcValue::Float(f) => LpcValue::Float(f - 1.0),
                        other => {
                            return Err(VmError::TypeError(format!(
                                "cannot decrement {}",
                                other.type_name()
                            )));
                        }
                    };
                    frame.locals[idx] = new_val.clone();
                    state.stack.push(new_val);
                }

                OpCode::PostIncLocal(idx) => {
                    let frame = state.call_stack.last_mut().unwrap();
                    let idx = idx as usize;
                    if idx >= frame.locals.len() {
                        frame.locals.resize(idx + 1, LpcValue::Nil);
                    }
                    let old_val = frame.locals[idx].clone();
                    let new_val = match &old_val {
                        LpcValue::Int(n) => LpcValue::Int(n + 1),
                        LpcValue::Float(f) => LpcValue::Float(f + 1.0),
                        other => {
                            return Err(VmError::TypeError(format!(
                                "cannot increment {}",
                                other.type_name()
                            )));
                        }
                    };
                    frame.locals[idx] = new_val;
                    state.stack.push(old_val);
                }

                OpCode::PostDecLocal(idx) => {
                    let frame = state.call_stack.last_mut().unwrap();
                    let idx = idx as usize;
                    if idx >= frame.locals.len() {
                        frame.locals.resize(idx + 1, LpcValue::Nil);
                    }
                    let old_val = frame.locals[idx].clone();
                    let new_val = match &old_val {
                        LpcValue::Int(n) => LpcValue::Int(n - 1),
                        LpcValue::Float(f) => LpcValue::Float(f - 1.0),
                        other => {
                            return Err(VmError::TypeError(format!(
                                "cannot decrement {}",
                                other.type_name()
                            )));
                        }
                    };
                    frame.locals[idx] = new_val;
                    state.stack.push(old_val);
                }

                // -----------------------------------------------------------
                // Tick control
                // -----------------------------------------------------------
                OpCode::CheckTicks(cost) => {
                    state.tick_counter = state
                        .tick_counter
                        .checked_sub(cost as u64)
                        .ok_or(VmError::TicksExhausted)?;
                }
            }
        }
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helper: pop
// ---------------------------------------------------------------------------

fn pop(stack: &mut Vec<LpcValue>) -> Result<LpcValue, VmError> {
    stack
        .pop()
        .ok_or_else(|| VmError::RuntimeError("stack underflow".into()))
}

// ---------------------------------------------------------------------------
// Arithmetic helpers
// ---------------------------------------------------------------------------

fn arith_add(a: LpcValue, b: LpcValue) -> Result<LpcValue, VmError> {
    match (a, b) {
        (LpcValue::Int(x), LpcValue::Int(y)) => Ok(LpcValue::Int(x.wrapping_add(y))),
        (LpcValue::Float(x), LpcValue::Float(y)) => Ok(LpcValue::Float(x + y)),
        (LpcValue::Int(x), LpcValue::Float(y)) => Ok(LpcValue::Float(x as f64 + y)),
        (LpcValue::Float(x), LpcValue::Int(y)) => Ok(LpcValue::Float(x + y as f64)),
        (LpcValue::String(mut x), LpcValue::String(y)) => {
            x.push_str(&y);
            Ok(LpcValue::String(x))
        }
        (LpcValue::Array(mut x), LpcValue::Array(y)) => {
            x.extend(y);
            Ok(LpcValue::Array(x))
        }
        (a, b) => Err(VmError::TypeError(format!(
            "cannot add {} and {}",
            a.type_name(),
            b.type_name()
        ))),
    }
}

fn arith_sub(a: LpcValue, b: LpcValue) -> Result<LpcValue, VmError> {
    match (a, b) {
        (LpcValue::Int(x), LpcValue::Int(y)) => Ok(LpcValue::Int(x.wrapping_sub(y))),
        (LpcValue::Float(x), LpcValue::Float(y)) => Ok(LpcValue::Float(x - y)),
        (LpcValue::Int(x), LpcValue::Float(y)) => Ok(LpcValue::Float(x as f64 - y)),
        (LpcValue::Float(x), LpcValue::Int(y)) => Ok(LpcValue::Float(x - y as f64)),
        (LpcValue::Array(x), LpcValue::Array(ref y)) => {
            let result: Vec<LpcValue> = x.into_iter().filter(|e| !y.contains(e)).collect();
            Ok(LpcValue::Array(result))
        }
        (a, b) => Err(VmError::TypeError(format!(
            "cannot subtract {} from {}",
            b.type_name(),
            a.type_name()
        ))),
    }
}

fn arith_mul(a: LpcValue, b: LpcValue) -> Result<LpcValue, VmError> {
    match (a, b) {
        (LpcValue::Int(x), LpcValue::Int(y)) => Ok(LpcValue::Int(x.wrapping_mul(y))),
        (LpcValue::Float(x), LpcValue::Float(y)) => Ok(LpcValue::Float(x * y)),
        (LpcValue::Int(x), LpcValue::Float(y)) => Ok(LpcValue::Float(x as f64 * y)),
        (LpcValue::Float(x), LpcValue::Int(y)) => Ok(LpcValue::Float(x * y as f64)),
        (a, b) => Err(VmError::TypeError(format!(
            "cannot multiply {} and {}",
            a.type_name(),
            b.type_name()
        ))),
    }
}

fn arith_div(a: LpcValue, b: LpcValue) -> Result<LpcValue, VmError> {
    match (a, b) {
        (LpcValue::Int(_), LpcValue::Int(0)) => Err(VmError::DivisionByZero),
        (LpcValue::Int(x), LpcValue::Int(y)) => Ok(LpcValue::Int(x.wrapping_div(y))),
        (LpcValue::Float(x), LpcValue::Float(y)) => {
            if y == 0.0 {
                Err(VmError::DivisionByZero)
            } else {
                Ok(LpcValue::Float(x / y))
            }
        }
        (LpcValue::Int(x), LpcValue::Float(y)) => {
            if y == 0.0 {
                Err(VmError::DivisionByZero)
            } else {
                Ok(LpcValue::Float(x as f64 / y))
            }
        }
        (LpcValue::Float(x), LpcValue::Int(y)) => {
            if y == 0 {
                Err(VmError::DivisionByZero)
            } else {
                Ok(LpcValue::Float(x / y as f64))
            }
        }
        (a, b) => Err(VmError::TypeError(format!(
            "cannot divide {} by {}",
            a.type_name(),
            b.type_name()
        ))),
    }
}

fn arith_mod(a: LpcValue, b: LpcValue) -> Result<LpcValue, VmError> {
    match (a, b) {
        (LpcValue::Int(_), LpcValue::Int(0)) => Err(VmError::DivisionByZero),
        (LpcValue::Int(x), LpcValue::Int(y)) => Ok(LpcValue::Int(x.wrapping_rem(y))),
        (a, b) => Err(VmError::TypeError(format!(
            "cannot modulo {} by {}",
            a.type_name(),
            b.type_name()
        ))),
    }
}

fn arith_neg(v: LpcValue) -> Result<LpcValue, VmError> {
    match v {
        LpcValue::Int(n) => Ok(LpcValue::Int(-n)),
        LpcValue::Float(f) => Ok(LpcValue::Float(-f)),
        other => Err(VmError::TypeError(format!(
            "cannot negate {}",
            other.type_name()
        ))),
    }
}

// ---------------------------------------------------------------------------
// Bitwise helpers (with array set operations)
// ---------------------------------------------------------------------------

fn bitwise_and(a: LpcValue, b: LpcValue) -> Result<LpcValue, VmError> {
    match (a, b) {
        (LpcValue::Int(x), LpcValue::Int(y)) => Ok(LpcValue::Int(x & y)),
        (LpcValue::Array(x), LpcValue::Array(ref y)) => {
            let result: Vec<LpcValue> = x.into_iter().filter(|e| y.contains(e)).collect();
            Ok(LpcValue::Array(result))
        }
        (a, b) => Err(VmError::TypeError(format!(
            "bitwise and requires int or array, got {} & {}",
            a.type_name(),
            b.type_name()
        ))),
    }
}

fn bitwise_or(a: LpcValue, b: LpcValue) -> Result<LpcValue, VmError> {
    match (a, b) {
        (LpcValue::Int(x), LpcValue::Int(y)) => Ok(LpcValue::Int(x | y)),
        (LpcValue::Array(mut x), LpcValue::Array(y)) => {
            for elem in y {
                if !x.contains(&elem) {
                    x.push(elem);
                }
            }
            Ok(LpcValue::Array(x))
        }
        (a, b) => Err(VmError::TypeError(format!(
            "bitwise or requires int or array, got {} | {}",
            a.type_name(),
            b.type_name()
        ))),
    }
}

// ---------------------------------------------------------------------------
// Comparison helpers
// ---------------------------------------------------------------------------

fn val_eq(a: &LpcValue, b: &LpcValue) -> bool {
    match (a, b) {
        (LpcValue::Nil, LpcValue::Nil) => true,
        (LpcValue::Int(x), LpcValue::Int(y)) => x == y,
        (LpcValue::Float(x), LpcValue::Float(y)) => x == y,
        (LpcValue::Int(x), LpcValue::Float(y)) => (*x as f64) == *y,
        (LpcValue::Float(x), LpcValue::Int(y)) => *x == (*y as f64),
        (LpcValue::String(x), LpcValue::String(y)) => x == y,
        (LpcValue::Object(x), LpcValue::Object(y)) => x.id == y.id && x.path == y.path,
        (LpcValue::Array(x), LpcValue::Array(y)) => x == y,
        (LpcValue::Mapping(x), LpcValue::Mapping(y)) => x == y,
        _ => false,
    }
}

fn val_cmp(a: &LpcValue, b: &LpcValue) -> Result<i64, VmError> {
    match (a, b) {
        (LpcValue::Int(x), LpcValue::Int(y)) => Ok(x.cmp(y) as i64),
        (LpcValue::Float(x), LpcValue::Float(y)) => {
            Ok(x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal) as i64)
        }
        (LpcValue::Int(x), LpcValue::Float(y)) => {
            let xf = *x as f64;
            Ok(xf.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal) as i64)
        }
        (LpcValue::Float(x), LpcValue::Int(y)) => {
            let yf = *y as f64;
            Ok(x.partial_cmp(&yf).unwrap_or(std::cmp::Ordering::Equal) as i64)
        }
        (LpcValue::String(x), LpcValue::String(y)) => Ok(x.cmp(y) as i64),
        _ => Err(VmError::TypeError(format!(
            "cannot compare {} and {}",
            a.type_name(),
            b.type_name()
        ))),
    }
}

// ---------------------------------------------------------------------------
// Collection helpers
// ---------------------------------------------------------------------------

fn index_value(collection: &LpcValue, index: &LpcValue) -> Result<LpcValue, VmError> {
    match (collection, index) {
        (LpcValue::Array(arr), LpcValue::Int(i)) => {
            let idx = normalize_index(*i, arr.len())?;
            Ok(arr[idx].clone())
        }
        (LpcValue::String(s), LpcValue::Int(i)) => {
            let chars: Vec<char> = s.chars().collect();
            let idx = normalize_index(*i, chars.len())?;
            Ok(LpcValue::String(chars[idx].to_string()))
        }
        (LpcValue::Mapping(m), key) => {
            for (k, v) in m {
                if val_eq(k, key) {
                    return Ok(v.clone());
                }
            }
            Ok(LpcValue::Nil)
        }
        _ => Err(VmError::TypeError(format!(
            "cannot index {} with {}",
            collection.type_name(),
            index.type_name()
        ))),
    }
}

fn index_assign(
    collection: &mut LpcValue,
    index: &LpcValue,
    value: LpcValue,
) -> Result<(), VmError> {
    match (collection, index) {
        (LpcValue::Array(ref mut arr), LpcValue::Int(i)) => {
            let idx = normalize_index(*i, arr.len())?;
            arr[idx] = value;
            Ok(())
        }
        (LpcValue::Mapping(ref mut m), key) => {
            for pair in m.iter_mut() {
                if val_eq(&pair.0, key) {
                    pair.1 = value;
                    return Ok(());
                }
            }
            m.push((key.clone(), value));
            Ok(())
        }
        (coll, _) => Err(VmError::TypeError(format!(
            "cannot index-assign {}",
            coll.type_name()
        ))),
    }
}

fn range_index(
    collection: &LpcValue,
    start: &LpcValue,
    end: &LpcValue,
) -> Result<LpcValue, VmError> {
    let start_i = match start {
        LpcValue::Int(i) => *i,
        LpcValue::Nil => 0,
        _ => {
            return Err(VmError::TypeError("range start must be int".into()));
        }
    };

    match collection {
        LpcValue::Array(arr) => {
            let end_i = match end {
                LpcValue::Int(i) => *i,
                LpcValue::Nil => arr.len() as i64 - 1,
                _ => {
                    return Err(VmError::TypeError("range end must be int".into()));
                }
            };
            let s = normalize_index(start_i, arr.len())?;
            let e = normalize_index(end_i, arr.len())?;
            if s > e {
                return Ok(LpcValue::Array(vec![]));
            }
            Ok(LpcValue::Array(arr[s..=e].to_vec()))
        }
        LpcValue::String(string) => {
            let chars: Vec<char> = string.chars().collect();
            let end_i = match end {
                LpcValue::Int(i) => *i,
                LpcValue::Nil => chars.len() as i64 - 1,
                _ => {
                    return Err(VmError::TypeError("range end must be int".into()));
                }
            };
            let s = normalize_index(start_i, chars.len())?;
            let e = normalize_index(end_i, chars.len())?;
            if s > e {
                return Ok(LpcValue::String(String::new()));
            }
            Ok(LpcValue::String(chars[s..=e].iter().collect()))
        }
        _ => Err(VmError::TypeError(format!(
            "cannot range-index {}",
            collection.type_name()
        ))),
    }
}

fn normalize_index(index: i64, len: usize) -> Result<usize, VmError> {
    let resolved = if index < 0 { len as i64 + index } else { index };
    if resolved < 0 || resolved as usize >= len {
        Err(VmError::IndexOutOfBounds { index, size: len })
    } else {
        Ok(resolved as usize)
    }
}

// ---------------------------------------------------------------------------
// Type cast helper
// ---------------------------------------------------------------------------

fn cast_value(val: LpcValue, type_id: u8) -> Result<LpcValue, VmError> {
    // Type IDs: 0=int, 1=float, 2=string, 3=object, 4=array, 5=mapping
    match type_id {
        0 => match val {
            LpcValue::Int(_) => Ok(val),
            LpcValue::Float(f) => Ok(LpcValue::Int(f as i64)),
            LpcValue::String(ref s) => Ok(LpcValue::Int(s.parse::<i64>().unwrap_or(0))),
            LpcValue::Nil => Ok(LpcValue::Int(0)),
            _ => Err(VmError::TypeError(format!(
                "cannot cast {} to int",
                val.type_name()
            ))),
        },
        1 => match val {
            LpcValue::Float(_) => Ok(val),
            LpcValue::Int(n) => Ok(LpcValue::Float(n as f64)),
            LpcValue::String(ref s) => Ok(LpcValue::Float(s.parse::<f64>().unwrap_or(0.0))),
            LpcValue::Nil => Ok(LpcValue::Float(0.0)),
            _ => Err(VmError::TypeError(format!(
                "cannot cast {} to float",
                val.type_name()
            ))),
        },
        2 => match val {
            LpcValue::String(_) => Ok(val),
            LpcValue::Int(n) => Ok(LpcValue::String(n.to_string())),
            LpcValue::Float(f) => Ok(LpcValue::String(f.to_string())),
            LpcValue::Nil => Ok(LpcValue::String(String::new())),
            _ => Err(VmError::TypeError(format!(
                "cannot cast {} to string",
                val.type_name()
            ))),
        },
        _ => Ok(val),
    }
}

// ---------------------------------------------------------------------------
// Compound assignment helpers
// ---------------------------------------------------------------------------

fn compound_assign_local(
    state: &mut ExecutionState,
    idx: u16,
    op: impl FnOnce(LpcValue, LpcValue) -> Result<LpcValue, VmError>,
) -> Result<(), VmError> {
    let rhs = pop(&mut state.stack)?;
    let frame = state.call_stack.last_mut().unwrap();
    let idx = idx as usize;
    if idx >= frame.locals.len() {
        frame.locals.resize(idx + 1, LpcValue::Nil);
    }
    let lhs = frame.locals[idx].clone();
    let result = op(lhs, rhs)?;
    frame.locals[idx] = result.clone();
    state.stack.push(result);
    Ok(())
}

fn compound_assign_global(
    state: &mut ExecutionState,
    object_table: &mut ObjectTable,
    idx: u16,
    op: impl FnOnce(LpcValue, LpcValue) -> Result<LpcValue, VmError>,
) -> Result<(), VmError> {
    let rhs = pop(&mut state.stack)?;
    let this_obj = &state.call_stack.last().unwrap().this_object;
    let lhs = object_table
        .get_global(this_obj, idx)
        .cloned()
        .unwrap_or(LpcValue::Nil);
    let result = op(lhs, rhs)?;
    let this_obj = state.call_stack.last().unwrap().this_object.clone();
    object_table
        .set_global(&this_obj, idx, result.clone())
        .map_err(VmError::from)?;
    state.stack.push(result);
    Ok(())
}
