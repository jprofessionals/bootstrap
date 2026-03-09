use lpc_vm::preprocessor::{PreprocessError, Preprocessor};

fn process(src: &str) -> Result<String, PreprocessError> {
    let mut pp = Preprocessor::new();
    pp.process(src, "test.c")
}

// =========================================================================
// Passthrough
// =========================================================================

#[test]
fn passthrough_no_directives() {
    let result = process("int x = 42;").unwrap();
    assert_eq!(result, "int x = 42;");
}

#[test]
fn passthrough_multiple_lines() {
    let result = process("line one\nline two").unwrap();
    assert_eq!(result, "line one\nline two");
}

// =========================================================================
// #define simple macro
// =========================================================================

#[test]
fn define_simple_macro() {
    let result = process("#define FOO 42\nint x = FOO;").unwrap();
    assert_eq!(result, "int x = 42;");
}

#[test]
fn define_empty_value() {
    let result = process("#define FOO\nFOO").unwrap();
    assert_eq!(result, "");
}

// =========================================================================
// #define with parameters
// =========================================================================

#[test]
fn define_parameterized_macro() {
    let result = process("#define MAX(a, b) ((a) > (b) ? (a) : (b))\nint z = MAX(3, 5);").unwrap();
    assert_eq!(result, "int z = ((3) > (5) ? (3) : (5));");
}

#[test]
fn define_parameterized_no_args() {
    let result = process("#define HELLO() 42\nint x = HELLO();").unwrap();
    assert_eq!(result, "int x = 42;");
}

// =========================================================================
// #undef
// =========================================================================

#[test]
fn undef_removes_macro() {
    let src = "#define FOO 1\n#undef FOO\nint x = FOO;";
    let result = process(src).unwrap();
    // FOO should not be expanded after #undef
    assert_eq!(result, "int x = FOO;");
}

// =========================================================================
// #ifdef
// =========================================================================

#[test]
fn ifdef_defined_includes_code() {
    let src = "#define DEBUG\n#ifdef DEBUG\nint debug = 1;\n#endif";
    let result = process(src).unwrap();
    assert_eq!(result, "int debug = 1;");
}

#[test]
fn ifdef_not_defined_skips_code() {
    let src = "#ifdef DEBUG\nint debug = 1;\n#endif\nint x = 2;";
    let result = process(src).unwrap();
    assert_eq!(result, "int x = 2;");
}

// =========================================================================
// #ifndef
// =========================================================================

#[test]
fn ifndef_not_defined_includes_code() {
    let src = "#ifndef RELEASE\nint debug = 1;\n#endif";
    let result = process(src).unwrap();
    assert_eq!(result, "int debug = 1;");
}

#[test]
fn ifndef_defined_skips_code() {
    let src = "#define RELEASE\n#ifndef RELEASE\nint debug = 1;\n#endif\nint x = 2;";
    let result = process(src).unwrap();
    assert_eq!(result, "int x = 2;");
}

// =========================================================================
// #if / #else / #endif
// =========================================================================

#[test]
fn if_true_branch() {
    let src = "#define VAL 1\n#if VAL\ntrue_branch\n#else\nfalse_branch\n#endif";
    let result = process(src).unwrap();
    assert_eq!(result, "true_branch");
}

#[test]
fn if_false_else_branch() {
    let src = "#if 0\ntrue_branch\n#else\nfalse_branch\n#endif";
    let result = process(src).unwrap();
    assert_eq!(result, "false_branch");
}

// =========================================================================
// #elif
// =========================================================================

#[test]
fn elif_selects_correct_branch() {
    let src = "#define X 2\n#if X == 1\nbranch1\n#elif X == 2\nbranch2\n#else\nbranch3\n#endif";
    let result = process(src).unwrap();
    assert_eq!(result, "branch2");
}

#[test]
fn elif_falls_through_to_else() {
    let src = "#if 0\nbranch1\n#elif 0\nbranch2\n#else\nbranch3\n#endif";
    let result = process(src).unwrap();
    assert_eq!(result, "branch3");
}

// =========================================================================
// Nested #ifdef
// =========================================================================

#[test]
fn nested_ifdef() {
    let src = "#define A\n#define B\n#ifdef A\n#ifdef B\nboth\n#endif\n#endif";
    let result = process(src).unwrap();
    assert_eq!(result, "both");
}

#[test]
fn nested_ifdef_outer_false() {
    let src = "#ifdef NOPE\n#ifdef ALSO_NOPE\nshould_not_appear\n#endif\n#endif\nvisible";
    let result = process(src).unwrap();
    assert_eq!(result, "visible");
}

// =========================================================================
// defined() operator
// =========================================================================

#[test]
fn defined_operator_true() {
    let src = "#define FOO 1\n#if defined(FOO)\nyes\n#endif";
    let result = process(src).unwrap();
    assert_eq!(result, "yes");
}

#[test]
fn defined_operator_false() {
    let src = "#if defined(BAR)\nyes\n#else\nno\n#endif";
    let result = process(src).unwrap();
    assert_eq!(result, "no");
}

// =========================================================================
// #include with resolver
// =========================================================================

#[test]
fn include_with_resolver() {
    let mut pp = Preprocessor::new();
    pp.set_include_resolver(Box::new(|path| {
        if path == "header.h" {
            Some("int included = 1;".to_string())
        } else {
            None
        }
    }));
    let result = pp.process("#include \"header.h\"\nint local = 2;", "test.c").unwrap();
    assert!(result.contains("int included = 1;"));
    assert!(result.contains("int local = 2;"));
}

#[test]
fn include_angle_brackets() {
    let mut pp = Preprocessor::new();
    pp.set_include_resolver(Box::new(|path| {
        if path == "sys/types.h" {
            Some("typedef int size_t;".to_string())
        } else {
            None
        }
    }));
    let result = pp.process("#include <sys/types.h>", "test.c").unwrap();
    assert!(result.contains("typedef int size_t;"));
}

#[test]
fn include_not_found_error() {
    let mut pp = Preprocessor::new();
    pp.set_include_resolver(Box::new(|_| None));
    let result = pp.process("#include \"nonexistent.h\"", "test.c");
    assert!(result.is_err());
    match result.unwrap_err() {
        PreprocessError::IncludeNotFound { path, .. } => {
            assert_eq!(path, "nonexistent.h");
        }
        other => panic!("expected IncludeNotFound, got: {:?}", other),
    }
}

// =========================================================================
// #error
// =========================================================================

#[test]
fn error_directive_produces_error() {
    let result = process("#error this is bad");
    assert!(result.is_err());
    match result.unwrap_err() {
        PreprocessError::UserError { message, .. } => {
            assert_eq!(message, "this is bad");
        }
        other => panic!("expected UserError, got: {:?}", other),
    }
}

#[test]
fn error_in_false_branch_not_triggered() {
    let result = process("#if 0\n#error should not fire\n#endif\nok");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "ok");
}

// =========================================================================
// Line continuations
// =========================================================================

#[test]
fn line_continuation_joins_lines() {
    let src = "#define LONG_MACRO \\\n  42\nint x = LONG_MACRO;";
    let result = process(src).unwrap();
    assert_eq!(result, "int x = 42;");
}

// =========================================================================
// Error cases
// =========================================================================

#[test]
fn unterminated_conditional_error() {
    let result = process("#ifdef FOO\nhello");
    assert!(result.is_err());
    match result.unwrap_err() {
        PreprocessError::UnterminatedConditional { .. } => {}
        other => panic!("expected UnterminatedConditional, got: {:?}", other),
    }
}

#[test]
fn else_without_if_error() {
    let result = process("#else\nhello\n#endif");
    assert!(result.is_err());
    match result.unwrap_err() {
        PreprocessError::ElseWithoutIf { .. } => {}
        other => panic!("expected ElseWithoutIf, got: {:?}", other),
    }
}

#[test]
fn endif_without_if_error() {
    let result = process("#endif");
    assert!(result.is_err());
    match result.unwrap_err() {
        PreprocessError::EndifWithoutIf { .. } => {}
        other => panic!("expected EndifWithoutIf, got: {:?}", other),
    }
}

#[test]
fn unknown_directive_error() {
    let result = process("#foobar");
    assert!(result.is_err());
    match result.unwrap_err() {
        PreprocessError::UnknownDirective { directive, .. } => {
            assert_eq!(directive, "foobar");
        }
        other => panic!("expected UnknownDirective, got: {:?}", other),
    }
}

// =========================================================================
// Pre-define macro via API
// =========================================================================

#[test]
fn predefine_macro() {
    let mut pp = Preprocessor::new();
    pp.define("VERSION", "3");
    let result = pp.process("int v = VERSION;", "test.c").unwrap();
    assert_eq!(result, "int v = 3;");
}

// =========================================================================
// Macro expansion does not match partial identifiers
// =========================================================================

#[test]
fn macro_word_boundary() {
    let mut pp = Preprocessor::new();
    pp.define("FOO", "replaced");
    let result = pp.process("FOOBAR FOO FOO2", "test.c").unwrap();
    // FOO should only be replaced where it's a whole word
    assert!(result.contains("FOOBAR"));
    assert!(result.contains("replaced"));
    assert!(result.contains("FOO2"));
}
