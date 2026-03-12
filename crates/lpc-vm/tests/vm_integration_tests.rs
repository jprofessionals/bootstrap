use lpc_vm::bytecode::LpcValue;
use lpc_vm::compiler::Compiler;
use lpc_vm::lexer::scanner::Scanner;
use lpc_vm::parser::Parser;
use lpc_vm::vm::Vm;

/// Compile and evaluate an LPC source by calling a named function.
fn eval(source: &str, func_name: &str, args: &[LpcValue]) -> Result<LpcValue, String> {
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_all().map_err(|e| format!("lex: {e}"))?;
    let mut parser = Parser::new(tokens);
    let program_ast = parser.parse_program().map_err(|e| format!("parse: {e}"))?;
    let mut compiler = Compiler::new();
    let mut compiled = compiler
        .compile(&program_ast)
        .map_err(|e| format!("compile: {e}"))?;
    compiled.path = "/test".to_string();
    let mut vm = Vm::new();
    let obj = vm.load_program(compiled);
    vm.call_function(&obj, func_name, args)
        .map_err(|e| format!("vm: {e}"))
}

/// Shortcut: eval with no arguments.
fn eval0(source: &str, func_name: &str) -> Result<LpcValue, String> {
    eval(source, func_name, &[])
}

// =========================================================================
// Arithmetic
// =========================================================================

#[test]
fn arithmetic_precedence() {
    let src = "int test() { return 2 + 3 * 4; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(14));
}

#[test]
fn arithmetic_subtraction() {
    let src = "int test() { return 10 - 3; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(7));
}

#[test]
fn arithmetic_division() {
    let src = "int test() { return 15 / 3; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(5));
}

#[test]
fn arithmetic_modulo() {
    let src = "int test() { return 17 % 5; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(2));
}

// =========================================================================
// String concatenation
// =========================================================================

#[test]
fn string_concat() {
    let src = r#"string test() { return "hello" + " " + "world"; }"#;
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_string(), Some("hello world"));
}

// =========================================================================
// If/else
// =========================================================================

#[test]
fn if_true_branch() {
    let src = "int test(int x) { if (x > 0) return 1; else return 0; }";
    let result = eval(src, "test", &[LpcValue::Int(5)]).unwrap();
    assert_eq!(result.as_int(), Some(1));
}

#[test]
fn if_false_branch() {
    let src = "int test(int x) { if (x > 0) return 1; else return 0; }";
    let result = eval(src, "test", &[LpcValue::Int(-1)]).unwrap();
    assert_eq!(result.as_int(), Some(0));
}

// =========================================================================
// While loop
// =========================================================================

#[test]
fn while_loop_sum() {
    let src = "
int test() {
    int sum;
    int i;
    sum = 0;
    i = 0;
    while (i < 10) {
        sum = sum + i;
        i = i + 1;
    }
    return sum;
}
";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(45));
}

// =========================================================================
// For loop
// =========================================================================

#[test]
fn for_loop_sum() {
    let src = "
int test() {
    int sum;
    int i;
    sum = 0;
    for (i = 0; i < 10; i = i + 1) {
        sum = sum + i;
    }
    return sum;
}
";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(45));
}

// =========================================================================
// Array creation and sizeof
// =========================================================================

#[test]
fn array_creation() {
    let src = "
mixed test() {
    mixed arr;
    arr = ({1, 2, 3});
    return arr;
}
";
    let result = eval0(src, "test").unwrap();
    let arr = result.as_array().expect("expected array");
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0].as_int(), Some(1));
    assert_eq!(arr[1].as_int(), Some(2));
    assert_eq!(arr[2].as_int(), Some(3));
}

// =========================================================================
// Mapping creation and index
// =========================================================================

#[test]
fn mapping_creation() {
    let src = r#"
mixed test() {
    mapping m;
    m = (["a": 1, "b": 2]);
    return m;
}
"#;
    let result = eval0(src, "test").unwrap();
    let m = result.as_mapping().expect("expected mapping");
    assert_eq!(m.len(), 2);
}

// =========================================================================
// Function calls between functions
// =========================================================================

#[test]
fn internal_function_call() {
    let src = "
int double(int x) {
    return x * 2;
}
int test() {
    return double(21);
}
";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(42));
}

// =========================================================================
// Variable assignment and access
// =========================================================================

#[test]
fn variable_assignment() {
    let src = "
int test() {
    int x;
    x = 42;
    return x;
}
";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(42));
}

// =========================================================================
// Increment/decrement
// =========================================================================

#[test]
fn pre_increment() {
    let src = "
int test() {
    int x;
    x = 5;
    ++x;
    return x;
}
";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(6));
}

#[test]
fn post_increment_returns_old_value() {
    let src = "
int test() {
    int x;
    int y;
    x = 5;
    y = x++;
    return y;
}
";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(5));
}

// =========================================================================
// Comparison operators
// =========================================================================

#[test]
fn comparison_equal() {
    let src = "int test() { return 5 == 5; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(1));
}

#[test]
fn comparison_not_equal() {
    let src = "int test() { return 5 != 3; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(1));
}

#[test]
fn comparison_less() {
    let src = "int test() { return 3 < 5; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(1));
}

#[test]
fn comparison_greater_eq() {
    let src = "int test() { return 5 >= 5; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(1));
}

// =========================================================================
// Logical operators
// =========================================================================

#[test]
fn logical_and_true() {
    let src = "int test() { return 1 && 1; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(1));
}

#[test]
fn logical_and_false() {
    let src = "int test() { return 1 && 0; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(0));
}

#[test]
fn logical_or_true() {
    let src = "int test() { return 0 || 1; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(1));
}

#[test]
fn logical_or_false() {
    let src = "int test() { return 0 || 0; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(0));
}

// =========================================================================
// Ternary expression
// =========================================================================

#[test]
fn ternary_true() {
    let src = "int test() { return 1 ? 42 : 0; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn ternary_false() {
    let src = "int test() { return 0 ? 42 : 99; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(99));
}

// =========================================================================
// Switch/case
// =========================================================================

#[test]
fn switch_case() {
    let src = "
int test(int x) {
    switch (x) {
        case 1:
            return 10;
        case 2:
            return 20;
        default:
            return 0;
    }
}
";
    assert_eq!(
        eval(src, "test", &[LpcValue::Int(1)]).unwrap().as_int(),
        Some(10)
    );
    assert_eq!(
        eval(src, "test", &[LpcValue::Int(2)]).unwrap().as_int(),
        Some(20)
    );
    assert_eq!(
        eval(src, "test", &[LpcValue::Int(99)]).unwrap().as_int(),
        Some(0)
    );
}

// =========================================================================
// Nested function calls
// =========================================================================

#[test]
fn nested_function_calls() {
    let src = "
int add(int a, int b) { return a + b; }
int mul(int a, int b) { return a * b; }
int test() { return add(mul(2, 3), mul(4, 5)); }
";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(26));
}

// =========================================================================
// Return nil
// =========================================================================

#[test]
fn void_function_returns_nil() {
    let src = "void test() { }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result, LpcValue::Nil);
}

// =========================================================================
// Integer literal zero
// =========================================================================

#[test]
fn integer_zero() {
    let src = "int test() { return 0; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(0));
}

// =========================================================================
// Negative number
// =========================================================================

#[test]
fn negative_number() {
    let src = "int test() { return -42; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(-42));
}

// =========================================================================
// String literal
// =========================================================================

#[test]
fn string_literal_return() {
    let src = r#"string test() { return "hello"; }"#;
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_string(), Some("hello"));
}

// =========================================================================
// Multiple arguments
// =========================================================================

#[test]
fn function_multiple_args() {
    let src = "int test(int a, int b, int c) { return a + b + c; }";
    let result = eval(
        src,
        "test",
        &[LpcValue::Int(10), LpcValue::Int(20), LpcValue::Int(30)],
    )
    .unwrap();
    assert_eq!(result.as_int(), Some(60));
}

// =========================================================================
// Compound assignment
// =========================================================================

#[test]
fn compound_add_assign() {
    let src = "
int test() {
    int x;
    x = 10;
    x += 5;
    return x;
}
";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(15));
}

// =========================================================================
// Boolean not
// =========================================================================

#[test]
fn boolean_not() {
    let src = "int test() { return !0; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(1));
}

#[test]
fn boolean_not_truthy() {
    let src = "int test() { return !42; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(0));
}

// =========================================================================
// Do-while loop
// =========================================================================

#[test]
fn do_while_loop() {
    let src = "
int test() {
    int x;
    x = 0;
    do {
        x = x + 1;
    } while (x < 5);
    return x;
}
";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(5));
}

// =========================================================================
// Bitwise operators
// =========================================================================

#[test]
fn bitwise_and() {
    let src = "int test() { return 0xFF & 0x0F; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(0x0F));
}

#[test]
fn bitwise_or() {
    let src = "int test() { return 0xF0 | 0x0F; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(0xFF));
}

#[test]
fn bitwise_xor() {
    let src = "int test() { return 0xFF ^ 0x0F; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(0xF0));
}

// =========================================================================
// Shift operators
// =========================================================================

#[test]
fn shift_left() {
    let src = "int test() { return 1 << 4; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(16));
}

#[test]
fn shift_right() {
    let src = "int test() { return 16 >> 2; }";
    let result = eval0(src, "test").unwrap();
    assert_eq!(result.as_int(), Some(4));
}
