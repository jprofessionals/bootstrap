use lpc_vm::ast::*;
use lpc_vm::lexer::scanner::Scanner;
use lpc_vm::parser::Parser;

/// Helper: tokenize source, then parse into a Program.
fn parse(src: &str) -> Program {
    let mut scanner = Scanner::new(src);
    let tokens = scanner.scan_all().expect("lexer failed");
    let mut parser = Parser::new(tokens);
    parser.parse_program().expect("parser failed")
}

// =========================================================================
// Empty program
// =========================================================================

#[test]
fn empty_program() {
    let program = parse("");
    assert!(program.inherits.is_empty());
    assert!(program.declarations.is_empty());
}

// =========================================================================
// Inherit declarations
// =========================================================================

#[test]
fn inherit_plain() {
    let program = parse(r#"inherit "/std/room";"#);
    assert_eq!(program.inherits.len(), 1);
    assert_eq!(program.inherits[0].path, "/std/room");
    assert_eq!(program.inherits[0].access, AccessModifier::Public);
    assert!(program.inherits[0].label.is_none());
}

#[test]
fn inherit_private() {
    let program = parse(r#"private inherit "/std/room";"#);
    assert_eq!(program.inherits.len(), 1);
    assert_eq!(program.inherits[0].access, AccessModifier::Private);
}

#[test]
fn inherit_labeled() {
    let program = parse(r#"inherit room "/std/room";"#);
    assert_eq!(program.inherits.len(), 1);
    assert_eq!(program.inherits[0].label.as_deref(), Some("room"));
    assert_eq!(program.inherits[0].path, "/std/room");
}

// =========================================================================
// Variable declarations
// =========================================================================

#[test]
fn variable_declaration_simple() {
    let program = parse("int x;");
    assert_eq!(program.declarations.len(), 1);
    match &program.declarations[0] {
        Declaration::Variable(v) => {
            assert_eq!(v.name, "x");
            assert_eq!(v.type_expr.base, BaseType::Int);
            assert_eq!(v.type_expr.array_depth, 0);
            assert!(v.initializer.is_none());
        }
        _ => panic!("expected variable declaration"),
    }
}

#[test]
fn variable_declaration_with_initializer() {
    let program = parse("int x = 42;");
    match &program.declarations[0] {
        Declaration::Variable(v) => {
            assert_eq!(v.name, "x");
            assert!(v.initializer.is_some());
        }
        _ => panic!("expected variable declaration"),
    }
}

#[test]
fn variable_string_type() {
    let program = parse("string name;");
    match &program.declarations[0] {
        Declaration::Variable(v) => {
            assert_eq!(v.type_expr.base, BaseType::String);
        }
        _ => panic!("expected variable declaration"),
    }
}

// =========================================================================
// Function declarations
// =========================================================================

#[test]
fn function_declaration_simple() {
    let program = parse("void create() { }");
    assert_eq!(program.declarations.len(), 1);
    match &program.declarations[0] {
        Declaration::Function(f) => {
            assert_eq!(f.name, "create");
            assert_eq!(f.return_type.base, BaseType::Void);
            assert!(f.params.is_empty());
            assert!(f.modifiers.is_empty());
        }
        _ => panic!("expected function declaration"),
    }
}

#[test]
fn function_with_params() {
    let program = parse("int add(int a, int b) { return a + b; }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            assert_eq!(f.name, "add");
            assert_eq!(f.return_type.base, BaseType::Int);
            assert_eq!(f.params.len(), 2);
            assert_eq!(f.params[0].name, "a");
            assert_eq!(f.params[0].type_expr.base, BaseType::Int);
            assert_eq!(f.params[1].name, "b");
        }
        _ => panic!("expected function declaration"),
    }
}

#[test]
fn function_with_modifiers() {
    let program = parse("private static int helper() { return 0; }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            assert!(f.modifiers.contains(&Modifier::Private));
            assert!(f.modifiers.contains(&Modifier::Static));
        }
        _ => panic!("expected function declaration"),
    }
}

#[test]
fn function_with_varargs() {
    let program = parse("varargs void log(string msg, mixed extra) { }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            assert!(f.modifiers.contains(&Modifier::Varargs));
        }
        _ => panic!("expected function declaration"),
    }
}

// =========================================================================
// Statements
// =========================================================================

#[test]
fn if_statement() {
    let program = parse("void test() { if (x) return 1; }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            assert!(matches!(&f.body[0], Stmt::If(_)));
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn if_else_statement() {
    let program = parse("void test() { if (x) return 1; else return 2; }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            if let Stmt::If(stmt) = &f.body[0] {
                assert!(stmt.else_branch.is_some());
            } else {
                panic!("expected if statement");
            }
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn while_statement() {
    let program = parse("void test() { while (x > 0) x--; }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            assert!(matches!(&f.body[0], Stmt::While(_)));
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn do_while_statement() {
    let program = parse("void test() { do { x++; } while (x < 10); }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            assert!(matches!(&f.body[0], Stmt::DoWhile(_)));
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn for_statement() {
    let program = parse("void test() { for (i = 0; i < 10; i++) sum += i; }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            assert!(matches!(&f.body[0], Stmt::For(_)));
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn switch_case_statement() {
    let program = parse(
        "void test() { switch(x) { case 1: return 10; case 2: return 20; default: return 0; } }",
    );
    match &program.declarations[0] {
        Declaration::Function(f) => {
            if let Stmt::Switch(sw) = &f.body[0] {
                assert_eq!(sw.cases.len(), 3);
                assert!(matches!(&sw.cases[2].label, CaseLabel::Default));
            } else {
                panic!("expected switch statement");
            }
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn return_statement_with_value() {
    let program = parse("int test() { return 42; }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            if let Stmt::Return(ret) = &f.body[0] {
                assert!(ret.value.is_some());
            } else {
                panic!("expected return statement");
            }
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn return_statement_without_value() {
    let program = parse("void test() { return; }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            if let Stmt::Return(ret) = &f.body[0] {
                assert!(ret.value.is_none());
            } else {
                panic!("expected return statement");
            }
        }
        _ => panic!("expected function"),
    }
}

// =========================================================================
// Expressions
// =========================================================================

#[test]
fn array_literal() {
    let program = parse("void test() { mixed a = ({1, 2, 3}); }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            // Should parse without error; body should contain a VarDecl
            assert!(!f.body.is_empty());
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn mapping_literal() {
    let program = parse(r#"void test() { mapping m = (["key": "val"]); }"#);
    match &program.declarations[0] {
        Declaration::Function(f) => {
            assert!(!f.body.is_empty());
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn binary_expression_precedence() {
    // 2 + 3 * 4 should parse as 2 + (3 * 4)
    let program = parse("int test() { return 2 + 3 * 4; }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            if let Stmt::Return(ret) = &f.body[0] {
                if let Some(Expr::Binary(bin)) = &ret.value {
                    assert_eq!(bin.op, BinaryOp::Add);
                    // Right side should be Mul
                    if let Expr::Binary(right) = bin.right.as_ref() {
                        assert_eq!(right.op, BinaryOp::Mul);
                    } else {
                        panic!("expected binary mul on right");
                    }
                } else {
                    panic!("expected binary expression");
                }
            } else {
                panic!("expected return statement");
            }
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn unary_negation() {
    let program = parse("int test() { return -x; }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            if let Stmt::Return(ret) = &f.body[0] {
                if let Some(Expr::Unary(u)) = &ret.value {
                    assert_eq!(u.op, UnaryOp::Neg);
                } else {
                    panic!("expected unary expression");
                }
            } else {
                panic!("expected return statement");
            }
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn unary_not() {
    let program = parse("int test() { return !x; }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            if let Stmt::Return(ret) = &f.body[0] {
                if let Some(Expr::Unary(u)) = &ret.value {
                    assert_eq!(u.op, UnaryOp::Not);
                } else {
                    panic!("expected unary not");
                }
            } else {
                panic!("expected return");
            }
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn call_expression() {
    let program = parse("void test() { func(1, 2); }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            if let Stmt::Expr(es) = &f.body[0] {
                assert!(matches!(&es.expr, Expr::Call(_)));
            } else {
                panic!("expected expression statement");
            }
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn call_other_expression() {
    let program = parse("void test() { obj->method(1); }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            if let Stmt::Expr(es) = &f.body[0] {
                if let Expr::CallOther(co) = &es.expr {
                    assert_eq!(co.method, "method");
                } else {
                    panic!("expected call_other expression");
                }
            } else {
                panic!("expected expression statement");
            }
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn parent_call_expression() {
    let program = parse("void test() { ::create(); }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            if let Stmt::Expr(es) = &f.body[0] {
                if let Expr::ParentCall(pc) = &es.expr {
                    assert_eq!(pc.function, "create");
                    assert!(pc.label.is_none());
                } else {
                    panic!("expected parent call expression");
                }
            } else {
                panic!("expected expression statement");
            }
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn index_expression() {
    let program = parse("void test() { arr[0]; }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            if let Stmt::Expr(es) = &f.body[0] {
                assert!(matches!(&es.expr, Expr::Index(_)));
            } else {
                panic!("expected expression statement");
            }
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn ternary_expression() {
    let program = parse("int test() { return x ? 1 : 0; }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            if let Stmt::Return(ret) = &f.body[0] {
                assert!(matches!(&ret.value, Some(Expr::Ternary(_))));
            } else {
                panic!("expected return statement");
            }
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn assignment_expression() {
    let program = parse("void test() { x = 5; }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            if let Stmt::Expr(es) = &f.body[0] {
                if let Expr::Assign(a) = &es.expr {
                    assert_eq!(a.op, AssignOp::Assign);
                } else {
                    panic!("expected assignment expression");
                }
            } else {
                panic!("expected expression statement");
            }
        }
        _ => panic!("expected function"),
    }
}

// =========================================================================
// Complete LPC program
// =========================================================================

#[test]
fn complete_lpc_program() {
    let src = r#"
inherit "/std/room";

int count;
string name = "test";

void create() {
    ::create();
    count = 0;
    name = "hello";
}

int get_count() {
    return count;
}
"#;
    let program = parse(src);
    assert_eq!(program.inherits.len(), 1);
    assert_eq!(program.inherits[0].path, "/std/room");
    // Should have 2 variables + 2 functions = 4 declarations
    let var_count = program
        .declarations
        .iter()
        .filter(|d| matches!(d, Declaration::Variable(_)))
        .count();
    let fn_count = program
        .declarations
        .iter()
        .filter(|d| matches!(d, Declaration::Function(_)))
        .count();
    assert_eq!(var_count, 2);
    assert_eq!(fn_count, 2);
}

// =========================================================================
// Pre-increment and post-increment
// =========================================================================

#[test]
fn pre_increment() {
    let program = parse("void test() { ++x; }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            if let Stmt::Expr(es) = &f.body[0] {
                if let Expr::Unary(u) = &es.expr {
                    assert_eq!(u.op, UnaryOp::PreIncrement);
                } else {
                    panic!("expected unary pre-increment");
                }
            } else {
                panic!("expected expression statement");
            }
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn post_increment() {
    let program = parse("void test() { x++; }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            if let Stmt::Expr(es) = &f.body[0] {
                assert!(matches!(&es.expr, Expr::PostIncrement(_, _)));
            } else {
                panic!("expected expression statement");
            }
        }
        _ => panic!("expected function"),
    }
}

// =========================================================================
// Compound assignment
// =========================================================================

#[test]
fn compound_assignment_plus() {
    let program = parse("void test() { x += 5; }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            if let Stmt::Expr(es) = &f.body[0] {
                if let Expr::Assign(a) = &es.expr {
                    assert_eq!(a.op, AssignOp::AddAssign);
                } else {
                    panic!("expected assignment");
                }
            } else {
                panic!("expected expression statement");
            }
        }
        _ => panic!("expected function"),
    }
}

// =========================================================================
// Comparison operators
// =========================================================================

#[test]
fn comparison_operators_parse() {
    let ops = vec![
        ("==", BinaryOp::Eq),
        ("!=", BinaryOp::NotEq),
        ("<", BinaryOp::Less),
        ("<=", BinaryOp::LessEq),
        (">", BinaryOp::Greater),
        (">=", BinaryOp::GreaterEq),
    ];
    for (op_str, expected_op) in ops {
        let src = format!("int test() {{ return x {} y; }}", op_str);
        let program = parse(&src);
        match &program.declarations[0] {
            Declaration::Function(f) => {
                if let Stmt::Return(ret) = &f.body[0] {
                    if let Some(Expr::Binary(bin)) = &ret.value {
                        assert_eq!(bin.op, expected_op, "operator: {}", op_str);
                    } else {
                        panic!("expected binary for {}", op_str);
                    }
                } else {
                    panic!("expected return for {}", op_str);
                }
            }
            _ => panic!("expected function for {}", op_str),
        }
    }
}

// =========================================================================
// Logical operators
// =========================================================================

#[test]
fn logical_and() {
    let program = parse("int test() { return x && y; }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            if let Stmt::Return(ret) = &f.body[0] {
                if let Some(Expr::Binary(bin)) = &ret.value {
                    assert_eq!(bin.op, BinaryOp::And);
                } else {
                    panic!("expected binary and");
                }
            } else {
                panic!("expected return");
            }
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn logical_or() {
    let program = parse("int test() { return x || y; }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            if let Stmt::Return(ret) = &f.body[0] {
                if let Some(Expr::Binary(bin)) = &ret.value {
                    assert_eq!(bin.op, BinaryOp::Or);
                } else {
                    panic!("expected binary or");
                }
            } else {
                panic!("expected return");
            }
        }
        _ => panic!("expected function"),
    }
}

// =========================================================================
// Nil literal
// =========================================================================

#[test]
fn nil_literal() {
    let program = parse("int test() { return nil; }");
    match &program.declarations[0] {
        Declaration::Function(f) => {
            if let Stmt::Return(ret) = &f.body[0] {
                assert!(matches!(&ret.value, Some(Expr::NilLiteral(_))));
            } else {
                panic!("expected return");
            }
        }
        _ => panic!("expected function"),
    }
}

// =========================================================================
// Multiple types
// =========================================================================

#[test]
fn all_base_types_parse() {
    let types = vec![
        ("int", BaseType::Int),
        ("float", BaseType::Float),
        ("string", BaseType::String),
        ("object", BaseType::Object),
        ("mapping", BaseType::Mapping),
        ("mixed", BaseType::Mixed),
        ("void", BaseType::Void),
    ];
    for (type_str, expected_type) in types {
        let src = format!("{} test() {{ }}", type_str);
        let program = parse(&src);
        match &program.declarations[0] {
            Declaration::Function(f) => {
                assert_eq!(f.return_type.base, expected_type, "type: {}", type_str);
            }
            _ => panic!("expected function for {}", type_str),
        }
    }
}
