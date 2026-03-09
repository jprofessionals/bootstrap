# Rust/LPC Adapter Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a combined Rust/LPC adapter with a full DGD-compatible LPC interpreter, Rust dynamic module system, and MOP integration — enabling seamless cross-language MUD gameplay.

**Architecture:** The LPC VM is a standalone Rust crate (`lpc-vm`). A separate adapter binary (`mud-adapter-lpc`) hosts the VM and communicates with the driver via MOP. The driver is extended with a state store, object broker, version tree, and diff-based reload system. Rust stdlib code compiles to granular `.so` modules that are hot-reloadable alongside LPC objects.

**Tech Stack:** Rust, tokio, rmp-serde (MessagePack), logos (lexer), MOP protocol, dlopen (dynamic loading), PostgreSQL (area databases)

**Reference:** See `docs/plans/2026-03-09-rust-lpc-adapter-design.md` for full design rationale.

---

## Phase 1: LPC VM — Lexer

Build the tokenizer for DGD-compatible LPC. The lexer handles all LPC tokens including
C-like operators, keywords, string/char/int/float literals, and identifiers.

### Task 1: Create lpc-vm crate scaffold

**Files:**
- Create: `crates/lpc-vm/Cargo.toml`
- Create: `crates/lpc-vm/src/lib.rs`
- Modify: `Cargo.toml` (workspace members)

**Step 1: Create the crate directory**

```bash
mkdir -p crates/lpc-vm/src
```

**Step 2: Write Cargo.toml**

```toml
# crates/lpc-vm/Cargo.toml
[package]
name = "lpc-vm"
version = "0.1.0"
edition = "2021"

[dependencies]
thiserror = { workspace = true }

[dev-dependencies]
```

**Step 3: Write lib.rs**

```rust
// crates/lpc-vm/src/lib.rs
pub mod lexer;
```

**Step 4: Create empty lexer module**

```rust
// crates/lpc-vm/src/lexer.rs
```

**Step 5: Add to workspace**

Add `"crates/lpc-vm"` to the `members` list in the root `Cargo.toml`.

**Step 6: Verify it compiles**

Run: `cargo check -p lpc-vm`
Expected: success (warning about empty file is fine)

**Step 7: Commit**

```bash
git add crates/lpc-vm/ Cargo.toml
git commit -m "feat: scaffold lpc-vm crate"
```

---

### Task 2: Implement LPC token types

**Files:**
- Create: `crates/lpc-vm/src/lexer/token.rs`
- Modify: `crates/lpc-vm/src/lexer.rs` → make it `mod token; pub use token::*;`

**Step 1: Define the Token enum and TokenKind**

```rust
// crates/lpc-vm/src/lexer/token.rs

/// Source location for error reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: u32,
    pub col: u32,
}

/// A single lexical token with its source location.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    pub text: String,
}

/// All LPC token kinds (DGD-compatible).
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    IntLiteral(i64),
    FloatLiteral(f64),
    StringLiteral(String),
    CharLiteral(char),

    // Identifier
    Identifier(String),

    // Keywords
    If,
    Else,
    While,
    Do,
    For,
    Switch,
    Case,
    Default,
    Break,
    Continue,
    Return,
    Inherit,
    Private,
    Static,
    Nomask,
    Atomic,
    Varargs,
    Int,
    Float,
    String_,     // "string" keyword (underscore to avoid Rust conflict)
    Object,
    Mapping,
    Mixed,
    Void,
    Nil,
    Rlimits,
    Catch,
    Sizeof,
    Typeof,
    New,

    // Operators
    Plus,         // +
    Minus,        // -
    Star,         // *
    Slash,        // /
    Percent,      // %
    Ampersand,    // &
    Pipe,         // |
    Caret,        // ^
    Tilde,        // ~
    Bang,         // !
    Assign,       // =
    Less,         // <
    Greater,      // >
    Dot,          // .
    Question,     // ?
    Colon,        // :

    // Multi-char operators
    PlusPlus,     // ++
    MinusMinus,   // --
    Arrow,        // ->
    ColonColon,   // ::
    DotDot,       // ..
    Ellipsis,     // ...
    EqualEqual,   // ==
    BangEqual,    // !=
    LessEqual,    // <=
    GreaterEqual, // >=
    AmpAmp,       // &&
    PipePipe,     // ||
    LessLess,     // <<
    GreaterGreater, // >>
    PlusAssign,   // +=
    MinusAssign,  // -=
    StarAssign,   // *=
    SlashAssign,  // /=
    PercentAssign,// %=
    AmpAssign,    // &=
    PipeAssign,   // |=
    CaretAssign,  // ^=
    LessLessAssign, // <<=
    GreaterGreaterAssign, // >>=

    // Delimiters
    LeftParen,    // (
    RightParen,   // )
    LeftBrace,    // {
    RightBrace,   // }
    LeftBracket,  // [
    RightBracket, // ]
    MappingOpen,  // ([
    MappingClose, // ])

    Semicolon,    // ;
    Comma,        // ,

    // Special
    Eof,
}
```

**Step 2: Write basic tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_kind_equality() {
        assert_eq!(TokenKind::If, TokenKind::If);
        assert_ne!(TokenKind::If, TokenKind::Else);
    }

    #[test]
    fn token_with_span() {
        let tok = Token {
            kind: TokenKind::IntLiteral(42),
            span: Span { start: 0, end: 2, line: 1, col: 1 },
            text: "42".into(),
        };
        assert_eq!(tok.kind, TokenKind::IntLiteral(42));
        assert_eq!(tok.span.line, 1);
    }
}
```

**Step 3: Update lexer.rs**

```rust
// crates/lpc-vm/src/lexer.rs
mod token;
pub use token::*;
```

**Step 4: Verify**

Run: `cargo test -p lpc-vm`
Expected: 2 tests pass

**Step 5: Commit**

```bash
git add crates/lpc-vm/
git commit -m "feat(lpc-vm): define LPC token types"
```

---

### Task 3: Implement the lexer

**Files:**
- Create: `crates/lpc-vm/src/lexer/scanner.rs`
- Modify: `crates/lpc-vm/src/lexer.rs`

Build a hand-written scanner that tokenizes LPC source code. Handles all DGD LPC
lexical elements: identifiers, keywords, integer literals (decimal, octal, hex),
float literals, string literals with escape sequences, char literals, all operators,
and comments (both `/* */` and `//`).

**Step 1: Write failing tests for basic tokens**

```rust
// In crates/lpc-vm/src/lexer/scanner.rs (tests module)
#[cfg(test)]
mod tests {
    use super::*;

    fn lex(input: &str) -> Vec<Token> {
        let mut scanner = Scanner::new(input);
        let mut tokens = Vec::new();
        loop {
            let tok = scanner.next_token().unwrap();
            if tok.kind == TokenKind::Eof {
                break;
            }
            tokens.push(tok);
        }
        tokens
    }

    fn kinds(input: &str) -> Vec<TokenKind> {
        lex(input).into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn empty_input() {
        assert_eq!(kinds(""), vec![]);
    }

    #[test]
    fn integer_literals() {
        assert_eq!(kinds("42"), vec![TokenKind::IntLiteral(42)]);
        assert_eq!(kinds("0x1F"), vec![TokenKind::IntLiteral(0x1F)]);
        assert_eq!(kinds("077"), vec![TokenKind::IntLiteral(0o77)]);
        assert_eq!(kinds("0"), vec![TokenKind::IntLiteral(0)]);
    }

    #[test]
    fn float_literals() {
        assert_eq!(kinds("3.14"), vec![TokenKind::FloatLiteral(3.14)]);
        assert_eq!(kinds("1.0e10"), vec![TokenKind::FloatLiteral(1.0e10)]);
        assert_eq!(kinds(".5"), vec![TokenKind::FloatLiteral(0.5)]);
    }

    #[test]
    fn string_literal() {
        assert_eq!(
            kinds(r#""hello""#),
            vec![TokenKind::StringLiteral("hello".into())]
        );
        assert_eq!(
            kinds(r#""line\n""#),
            vec![TokenKind::StringLiteral("line\n".into())]
        );
    }

    #[test]
    fn char_literal() {
        assert_eq!(kinds("'a'"), vec![TokenKind::CharLiteral('a')]);
        assert_eq!(kinds(r"'\n'"), vec![TokenKind::CharLiteral('\n')]);
    }

    #[test]
    fn keywords() {
        assert_eq!(kinds("if"), vec![TokenKind::If]);
        assert_eq!(kinds("else"), vec![TokenKind::Else]);
        assert_eq!(kinds("while"), vec![TokenKind::While]);
        assert_eq!(kinds("return"), vec![TokenKind::Return]);
        assert_eq!(kinds("inherit"), vec![TokenKind::Inherit]);
        assert_eq!(kinds("int"), vec![TokenKind::Int]);
        assert_eq!(kinds("string"), vec![TokenKind::String_]);
        assert_eq!(kinds("object"), vec![TokenKind::Object]);
        assert_eq!(kinds("mapping"), vec![TokenKind::Mapping]);
        assert_eq!(kinds("mixed"), vec![TokenKind::Mixed]);
        assert_eq!(kinds("void"), vec![TokenKind::Void]);
        assert_eq!(kinds("nil"), vec![TokenKind::Nil]);
        assert_eq!(kinds("static"), vec![TokenKind::Static]);
        assert_eq!(kinds("private"), vec![TokenKind::Private]);
        assert_eq!(kinds("nomask"), vec![TokenKind::Nomask]);
        assert_eq!(kinds("atomic"), vec![TokenKind::Atomic]);
        assert_eq!(kinds("varargs"), vec![TokenKind::Varargs]);
        assert_eq!(kinds("rlimits"), vec![TokenKind::Rlimits]);
    }

    #[test]
    fn identifiers() {
        assert_eq!(kinds("foo"), vec![TokenKind::Identifier("foo".into())]);
        assert_eq!(kinds("_bar"), vec![TokenKind::Identifier("_bar".into())]);
        assert_eq!(kinds("x123"), vec![TokenKind::Identifier("x123".into())]);
    }

    #[test]
    fn operators() {
        assert_eq!(kinds("+ - * /"), vec![
            TokenKind::Plus, TokenKind::Minus, TokenKind::Star, TokenKind::Slash
        ]);
        assert_eq!(kinds("== != <= >="), vec![
            TokenKind::EqualEqual, TokenKind::BangEqual,
            TokenKind::LessEqual, TokenKind::GreaterEqual,
        ]);
        assert_eq!(kinds("&& ||"), vec![TokenKind::AmpAmp, TokenKind::PipePipe]);
        assert_eq!(kinds("++ --"), vec![TokenKind::PlusPlus, TokenKind::MinusMinus]);
        assert_eq!(kinds("->"), vec![TokenKind::Arrow]);
        assert_eq!(kinds("::"), vec![TokenKind::ColonColon]);
        assert_eq!(kinds("..."), vec![TokenKind::Ellipsis]);
    }

    #[test]
    fn compound_assignment() {
        assert_eq!(kinds("+= -= *= /= %="), vec![
            TokenKind::PlusAssign, TokenKind::MinusAssign,
            TokenKind::StarAssign, TokenKind::SlashAssign,
            TokenKind::PercentAssign,
        ]);
    }

    #[test]
    fn delimiters() {
        assert_eq!(kinds("( ) { } [ ]"), vec![
            TokenKind::LeftParen, TokenKind::RightParen,
            TokenKind::LeftBrace, TokenKind::RightBrace,
            TokenKind::LeftBracket, TokenKind::RightBracket,
        ]);
        assert_eq!(kinds("(["), vec![TokenKind::MappingOpen]);
        assert_eq!(kinds("])"), vec![TokenKind::MappingClose]);
    }

    #[test]
    fn comments_skipped() {
        assert_eq!(kinds("42 /* comment */ 7"), vec![
            TokenKind::IntLiteral(42), TokenKind::IntLiteral(7)
        ]);
        assert_eq!(kinds("42 // line comment\n7"), vec![
            TokenKind::IntLiteral(42), TokenKind::IntLiteral(7)
        ]);
    }

    #[test]
    fn mapping_literal() {
        // ([ "key": value ])
        assert_eq!(kinds(r#"([ "key" : 1 ])"#), vec![
            TokenKind::MappingOpen,
            TokenKind::StringLiteral("key".into()),
            TokenKind::Colon,
            TokenKind::IntLiteral(1),
            TokenKind::MappingClose,
        ]);
    }

    #[test]
    fn lpc_function_signature() {
        assert_eq!(kinds("void create()"), vec![
            TokenKind::Void,
            TokenKind::Identifier("create".into()),
            TokenKind::LeftParen,
            TokenKind::RightParen,
        ]);
    }

    #[test]
    fn inherit_statement() {
        assert_eq!(kinds(r#"inherit "/std/room";"#), vec![
            TokenKind::Inherit,
            TokenKind::StringLiteral("/std/room".into()),
            TokenKind::Semicolon,
        ]);
    }

    #[test]
    fn span_tracking() {
        let tokens = lex("x + y");
        assert_eq!(tokens[0].span.col, 1);
        assert_eq!(tokens[1].span.col, 3);
        assert_eq!(tokens[2].span.col, 5);
    }

    #[test]
    fn multiline_span_tracking() {
        let tokens = lex("x\ny");
        assert_eq!(tokens[0].span.line, 1);
        assert_eq!(tokens[1].span.line, 2);
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p lpc-vm`
Expected: compile error (Scanner doesn't exist yet)

**Step 3: Implement Scanner**

```rust
// crates/lpc-vm/src/lexer/scanner.rs

use super::token::*;

#[derive(Debug, thiserror::Error)]
pub enum LexError {
    #[error("unexpected character '{ch}' at line {line}, col {col}")]
    UnexpectedChar { ch: char, line: u32, col: u32 },
    #[error("unterminated string literal at line {line}")]
    UnterminatedString { line: u32 },
    #[error("unterminated char literal at line {line}")]
    UnterminatedChar { line: u32 },
    #[error("unterminated block comment at line {line}")]
    UnterminatedComment { line: u32 },
    #[error("invalid escape sequence '\\{ch}' at line {line}")]
    InvalidEscape { ch: char, line: u32 },
    #[error("invalid number literal at line {line}")]
    InvalidNumber { line: u32 },
}

pub struct Scanner<'a> {
    source: &'a [u8],
    pos: usize,
    line: u32,
    col: u32,
}
```

The implementation should handle:
- Whitespace skipping (space, tab, newline, carriage return)
- Line comments (`//` to end of line)
- Block comments (`/* ... */`, including nested)
- Integer literals: decimal, hex (`0x`/`0X`), octal (`0` prefix)
- Float literals: `3.14`, `.5`, `1e10`, `1.5e-3`
- String literals with escape sequences: `\n`, `\t`, `\\`, `\"`, `\0`, `\x41`
- Char literals with the same escape sequences
- All single and multi-character operators (use longest match)
- `([` and `])` as mapping delimiters
- Keywords vs identifiers (keyword lookup table)
- Span tracking (line and column)

The `next_token()` method returns `Result<Token, LexError>`.

**Step 4: Run tests to verify they pass**

Run: `cargo test -p lpc-vm`
Expected: all lexer tests pass

**Step 5: Update lexer.rs**

```rust
// crates/lpc-vm/src/lexer.rs
mod token;
mod scanner;
pub use token::*;
pub use scanner::*;
```

**Step 6: Commit**

```bash
git add crates/lpc-vm/
git commit -m "feat(lpc-vm): implement LPC lexer with full DGD token support"
```

---

### Task 4: Implement the preprocessor

**Files:**
- Create: `crates/lpc-vm/src/preprocessor.rs`
- Modify: `crates/lpc-vm/src/lib.rs`

LPC uses a C-style preprocessor. Implement: `#include`, `#define` (with and without
parameters), `#undef`, `#ifdef`/`#ifndef`/`#if`/`#elif`/`#else`/`#endif`, `#pragma`,
`#line`, `#error`. The preprocessor operates on text before lexing.

**Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn preprocess(input: &str) -> Result<String, PreprocessError> {
        let mut pp = Preprocessor::new();
        pp.process(input, "test.c")
    }

    fn preprocess_with_includes(
        input: &str,
        resolver: &dyn Fn(&str) -> Option<String>,
    ) -> Result<String, PreprocessError> {
        let mut pp = Preprocessor::new();
        pp.set_include_resolver(resolver);
        pp.process(input, "test.c")
    }

    #[test]
    fn passthrough_no_directives() {
        let result = preprocess("int x;").unwrap();
        assert_eq!(result.trim(), "int x;");
    }

    #[test]
    fn define_simple() {
        let result = preprocess("#define FOO 42\nint x = FOO;").unwrap();
        assert!(result.contains("int x = 42;"));
    }

    #[test]
    fn define_with_params() {
        let result = preprocess("#define MAX(a,b) ((a)>(b)?(a):(b))\nint x = MAX(1,2);").unwrap();
        assert!(result.contains("int x = ((1)>(2)?(1):(2));"));
    }

    #[test]
    fn undef() {
        let result = preprocess("#define FOO 1\n#undef FOO\nint x = FOO;").unwrap();
        assert!(result.contains("int x = FOO;"));
    }

    #[test]
    fn ifdef_defined() {
        let result = preprocess("#define FOO\n#ifdef FOO\nint x;\n#endif").unwrap();
        assert!(result.contains("int x;"));
    }

    #[test]
    fn ifdef_not_defined() {
        let result = preprocess("#ifdef FOO\nint x;\n#endif").unwrap();
        assert!(!result.contains("int x;"));
    }

    #[test]
    fn ifndef() {
        let result = preprocess("#ifndef FOO\nint x;\n#endif").unwrap();
        assert!(result.contains("int x;"));
    }

    #[test]
    fn if_else() {
        let input = "#define FOO 1\n#if FOO\nint a;\n#else\nint b;\n#endif";
        let result = preprocess(input).unwrap();
        assert!(result.contains("int a;"));
        assert!(!result.contains("int b;"));
    }

    #[test]
    fn elif() {
        let input = "#define X 2\n#if X == 1\na;\n#elif X == 2\nb;\n#else\nc;\n#endif";
        let result = preprocess(input).unwrap();
        assert!(result.contains("b;"));
        assert!(!result.contains("a;"));
        assert!(!result.contains("c;"));
    }

    #[test]
    fn include_local() {
        let resolver = |path: &str| -> Option<String> {
            if path == "header.h" {
                Some("int imported;".into())
            } else {
                None
            }
        };
        let result = preprocess_with_includes(
            "#include \"header.h\"\nint x;",
            &resolver,
        ).unwrap();
        assert!(result.contains("int imported;"));
        assert!(result.contains("int x;"));
    }

    #[test]
    fn include_system() {
        let resolver = |path: &str| -> Option<String> {
            if path == "system.h" {
                Some("#define SYS 1".into())
            } else {
                None
            }
        };
        let result = preprocess_with_includes(
            "#include <system.h>\nint x = SYS;",
            &resolver,
        ).unwrap();
        assert!(result.contains("int x = 1;"));
    }

    #[test]
    fn error_directive() {
        let result = preprocess("#error something went wrong");
        assert!(result.is_err());
    }

    #[test]
    fn nested_ifdef() {
        let input = "#define A\n#define B\n#ifdef A\n#ifdef B\nyes;\n#endif\n#endif";
        let result = preprocess(input).unwrap();
        assert!(result.contains("yes;"));
    }

    #[test]
    fn defined_operator() {
        let input = "#define FOO\n#if defined(FOO)\nyes;\n#endif";
        let result = preprocess(input).unwrap();
        assert!(result.contains("yes;"));
    }

    #[test]
    fn line_continuations() {
        let input = "#define LONG \\\n    value\nint x = LONG;";
        let result = preprocess(input).unwrap();
        assert!(result.contains("int x = value;"));
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p lpc-vm preprocessor`
Expected: compile error

**Step 3: Implement Preprocessor**

The preprocessor struct should hold:
- A `HashMap<String, MacroDef>` for defined macros
- An include resolver callback
- A conditional stack for `#if`/`#ifdef` nesting

Process line by line. Lines starting with `#` (after whitespace) are directives.
Non-directive lines have macros expanded. Lines inside a false conditional branch
are skipped.

**Step 4: Run tests**

Run: `cargo test -p lpc-vm preprocessor`
Expected: all pass

**Step 5: Commit**

```bash
git add crates/lpc-vm/
git commit -m "feat(lpc-vm): implement C-style preprocessor"
```

---

## Phase 2: LPC VM — Parser

### Task 5: Define the AST

**Files:**
- Create: `crates/lpc-vm/src/ast.rs`
- Modify: `crates/lpc-vm/src/lib.rs`

Define AST node types for all LPC constructs: programs, inherit declarations, function
definitions, variable declarations, statements (if, while, do, for, switch, return,
rlimits, catch, block), and expressions (binary, unary, call, call_other/arrow, index,
member, assignment, ternary, mapping literal, array literal, cast, sizeof, typeof,
new_object, catch).

```rust
// crates/lpc-vm/src/ast.rs

/// A complete LPC source file.
pub struct Program {
    pub inherits: Vec<InheritDecl>,
    pub declarations: Vec<Declaration>,
}

pub struct InheritDecl {
    pub label: Option<String>,
    pub path: String,
    pub access: AccessModifier,
    pub span: Span,
}

pub enum AccessModifier { Public, Private }

pub enum Declaration {
    Function(FunctionDecl),
    Variable(VarDecl),
}

pub struct FunctionDecl {
    pub modifiers: Vec<Modifier>,
    pub return_type: TypeExpr,
    pub name: String,
    pub params: Vec<Param>,
    pub body: Block,
    pub span: Span,
}

pub enum Modifier { Private, Static, Nomask, Atomic, Varargs }

pub struct TypeExpr {
    pub base: BaseType,
    pub array_depth: u32,  // int* = 1, int** = 2
}

pub enum BaseType { Int, Float, String, Object, Mapping, Mixed, Void }

pub struct Param {
    pub type_expr: TypeExpr,
    pub name: String,
    pub varargs: bool,
}

pub struct VarDecl {
    pub modifiers: Vec<Modifier>,
    pub type_expr: TypeExpr,
    pub name: String,
    pub initializer: Option<Expr>,
    pub span: Span,
}

pub type Block = Vec<Stmt>;

pub enum Stmt {
    Expr(Expr),
    VarDecl(VarDecl),
    If { condition: Expr, then_branch: Block, else_branch: Option<Block>, span: Span },
    While { condition: Expr, body: Block, span: Span },
    DoWhile { body: Block, condition: Expr, span: Span },
    For { init: Option<Box<Stmt>>, condition: Option<Expr>, update: Option<Expr>, body: Block, span: Span },
    Switch { expr: Expr, cases: Vec<SwitchCase>, span: Span },
    Return { value: Option<Expr>, span: Span },
    Break { span: Span },
    Continue { span: Span },
    Block(Block),
    Rlimits { ticks: Expr, stack: Expr, body: Block, span: Span },
    Catch { body: Block, span: Span },
}

pub struct SwitchCase {
    pub label: CaseLabel,
    pub body: Block,
}

pub enum CaseLabel {
    Value(Expr),
    Range(Expr, Expr),
    Default,
}

pub enum Expr {
    IntLiteral(i64, Span),
    FloatLiteral(f64, Span),
    StringLiteral(String, Span),
    CharLiteral(char, Span),
    NilLiteral(Span),
    Identifier(String, Span),
    Binary { left: Box<Expr>, op: BinaryOp, right: Box<Expr>, span: Span },
    Unary { op: UnaryOp, operand: Box<Expr>, span: Span },
    PostIncrement { operand: Box<Expr>, span: Span },
    PostDecrement { operand: Box<Expr>, span: Span },
    Assign { target: Box<Expr>, op: AssignOp, value: Box<Expr>, span: Span },
    Ternary { condition: Box<Expr>, then_expr: Box<Expr>, else_expr: Box<Expr>, span: Span },
    Call { function: Box<Expr>, args: Vec<Expr>, span: Span },
    CallOther { object: Box<Expr>, method: String, args: Vec<Expr>, span: Span },
    ParentCall { label: Option<String>, function: String, args: Vec<Expr>, span: Span },
    Index { object: Box<Expr>, index: Box<Expr>, span: Span },
    Range { object: Box<Expr>, start: Option<Box<Expr>>, end: Option<Box<Expr>>, span: Span },
    ArrayLiteral(Vec<Expr>, Span),
    MappingLiteral(Vec<(Expr, Expr)>, Span),
    Cast { type_expr: TypeExpr, expr: Box<Expr>, span: Span },
    Sizeof(Box<Expr>, Span),
    Typeof(Box<Expr>, Span),
    NewObject(Box<Expr>, Span),
    CatchExpr { body: Box<Expr>, span: Span },
}

pub enum BinaryOp {
    Add, Sub, Mul, Div, Mod,
    Eq, Ne, Lt, Gt, Le, Ge,
    And, Or,
    BitAnd, BitOr, BitXor,
    Shl, Shr,
}

pub enum UnaryOp { Neg, Not, BitNot, PreIncrement, PreDecrement }

pub enum AssignOp {
    Assign, AddAssign, SubAssign, MulAssign, DivAssign, ModAssign,
    AndAssign, OrAssign, XorAssign, ShlAssign, ShrAssign,
}
```

**Step 1: Create ast.rs with the types above and basic tests**
**Step 2: Verify it compiles: `cargo check -p lpc-vm`**
**Step 3: Commit**

---

### Task 6: Implement the parser (expressions)

**Files:**
- Create: `crates/lpc-vm/src/parser.rs`
- Modify: `crates/lpc-vm/src/lib.rs`

Implement a recursive-descent parser for LPC expressions, using Pratt parsing for
operator precedence. The parser consumes tokens from the lexer and produces AST nodes.

Follow DGD's operator precedence (same as C):
1. Primary: literals, identifiers, parenthesized, array/mapping literals
2. Postfix: `()`, `[]`, `->`, `++`, `--`
3. Unary: `-`, `!`, `~`, `++`, `--`, cast, sizeof, typeof, new
4. Multiplicative: `*`, `/`, `%`
5. Additive: `+`, `-`
6. Shift: `<<`, `>>`
7. Relational: `<`, `>`, `<=`, `>=`
8. Equality: `==`, `!=`
9. Bitwise AND: `&`
10. Bitwise XOR: `^`
11. Bitwise OR: `|`
12. Logical AND: `&&`
13. Logical OR: `||`
14. Ternary: `? :`
15. Assignment: `=`, `+=`, `-=`, etc.
16. Comma: `,`

Write tests for each precedence level and associativity. Test edge cases like
`a->b(c)`, `::create()`, `label::func()`, `([key:val])`, `({1,2,3})`.

**Step 1: Write failing expression tests**
**Step 2: Implement parser expression handling**
**Step 3: Run tests**
**Step 4: Commit**

---

### Task 7: Implement the parser (statements and declarations)

**Files:**
- Modify: `crates/lpc-vm/src/parser.rs`

Extend the parser with:
- Variable declarations (with type, modifiers, optional initializer)
- Function declarations (modifiers, return type, params, body)
- `inherit` declarations (with optional label and access)
- Statement parsing: if/else, while, do/while, for, switch/case, return, break,
  continue, block, rlimits, catch
- Full program parsing (a sequence of inherits then declarations)

DGD quirk: no assignment in declarations (e.g., `int x = 5;` is valid but only as
a statement, not in DGD-strict mode — we support it as a common LPC extension).

Write tests for complete LPC programs:

```c
inherit "/std/room";

private string description;

void create() {
    ::create();
    description = "A dark room";
}

string get_description() {
    return description;
}
```

**Step 1: Write failing tests for statements and programs**
**Step 2: Implement statement and declaration parsing**
**Step 3: Run tests**
**Step 4: Commit**

---

## Phase 3: LPC VM — Compiler and Bytecode VM

### Task 8: Design bytecode instruction set

**Files:**
- Create: `crates/lpc-vm/src/bytecode.rs`
- Modify: `crates/lpc-vm/src/lib.rs`

Define the bytecode instruction set for the LPC VM. Stack-based architecture.

```rust
pub enum OpCode {
    // Stack manipulation
    Push(LpcValue),       // Push constant
    Pop,                  // Discard top
    Dup,                  // Duplicate top

    // Arithmetic (pop 2, push 1)
    Add, Sub, Mul, Div, Mod, Neg,

    // Comparison (pop 2, push bool)
    Eq, Ne, Lt, Gt, Le, Ge,

    // Logical
    Not, And, Or,

    // Bitwise
    BitAnd, BitOr, BitXor, BitNot, Shl, Shr,

    // Variables
    GetLocal(u16),        // Push local variable by slot index
    SetLocal(u16),        // Pop and store to local slot
    GetGlobal(u16),       // Push global variable by index
    SetGlobal(u16),       // Pop and store to global

    // Control flow
    Jump(i32),            // Unconditional jump (relative offset)
    JumpIfFalse(i32),     // Conditional jump
    JumpIfTrue(i32),

    // Functions
    Call(u16),            // Call function (arg count on stack)
    CallOther(u16),       // obj->method(args) (arg count)
    CallParent(u16),      // ::function(args)
    Return,               // Return from function (top of stack = return value)
    ReturnVoid,           // Return nil

    // Objects
    CloneObject,          // clone_object(master)
    NewObject,            // new_object(master)
    DestructObject,       // destruct_object(obj)
    ThisObject,           // Push this_object()

    // Collections
    MakeArray(u16),       // Pop N elements, push array
    MakeMapping(u16),     // Pop 2*N elements (key,val pairs), push mapping
    Index,                // Pop index, pop collection, push element
    IndexAssign,          // Pop value, pop index, pop collection, assign
    RangeIndex,           // Pop end, pop start, pop collection, push slice
    Sizeof,               // Pop collection, push size

    // Type operations
    TypeOf,               // Pop value, push type integer
    CastType(u8),         // Pop value, push casted value

    // Kfun calls
    CallKfun(u16, u8),    // kfun_id, arg_count

    // Tick/resource control
    CheckTicks(u32),      // Decrement tick counter, error if exhausted
}

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

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectRef {
    pub id: u64,
    pub path: String,
}

/// A compiled function.
pub struct CompiledFunction {
    pub name: String,
    pub arity: u16,
    pub local_count: u16,
    pub code: Vec<OpCode>,
    pub is_varargs: bool,
}

/// A compiled LPC program (one .c file).
pub struct CompiledProgram {
    pub path: String,
    pub inherits: Vec<String>,
    pub functions: Vec<CompiledFunction>,
    pub global_count: u16,
    pub global_names: Vec<String>,
}
```

**Step 1: Define types with basic tests**
**Step 2: Verify compilation**
**Step 3: Commit**

---

### Task 9: Implement the bytecode compiler

**Files:**
- Create: `crates/lpc-vm/src/compiler.rs`
- Modify: `crates/lpc-vm/src/lib.rs`

Walk the AST and emit bytecode. Handle:
- Expression compilation (literals, operators, calls)
- Variable resolution (locals by slot index, globals by index)
- Control flow (if/else → JumpIfFalse/Jump, while → Jump back, for, switch)
- Function compilation (allocate local slots, emit Return)
- Inherit tracking (record inherited paths for later resolution)

Test by compiling simple functions and verifying the emitted bytecode sequence.

**Step 1: Write failing tests (compile expression, verify opcodes)**
**Step 2: Implement compiler**
**Step 3: Run tests**
**Step 4: Commit**

---

### Task 10: Implement the bytecode VM

**Files:**
- Create: `crates/lpc-vm/src/vm.rs`
- Modify: `crates/lpc-vm/src/lib.rs`

Stack-based VM that executes bytecodes. Maintains:
- Value stack
- Call stack (frames with return address, local variable slots)
- Tick counter (decremented on each instruction, error if exhausted)
- Stack depth limit

Test by compiling and running simple LPC programs end-to-end:

```rust
#[test]
fn eval_arithmetic() {
    let result = eval("int f() { return 2 + 3 * 4; }");
    assert_eq!(result, LpcValue::Int(14));
}

#[test]
fn eval_string_concat() {
    let result = eval(r#"string f() { return "hello" + " " + "world"; }"#);
    assert_eq!(result, LpcValue::String("hello world".into()));
}

#[test]
fn eval_if_else() {
    let result = eval("int f(int x) { if (x > 0) return 1; else return -1; }");
    // call with x=5
    assert_eq!(call_with(result_program, "f", vec![LpcValue::Int(5)]), LpcValue::Int(1));
}

#[test]
fn eval_while_loop() {
    let result = eval("int f() { int i = 0; int sum = 0; while (i < 10) { sum += i; i++; } return sum; }");
    assert_eq!(result, LpcValue::Int(45));
}

#[test]
fn eval_array_operations() {
    let result = eval("int f() { int *a = ({1,2,3}); return sizeof(a); }");
    assert_eq!(result, LpcValue::Int(3));
}

#[test]
fn eval_mapping() {
    let result = eval(r#"string f() { mapping m = (["key":"value"]); return m["key"]; }"#);
    assert_eq!(result, LpcValue::String("value".into()));
}
```

**Step 1: Write failing end-to-end tests**
**Step 2: Implement VM execution loop**
**Step 3: Run tests**
**Step 4: Commit**

---

## Phase 4: LPC VM — Object Model

### Task 11: Implement the object table and lifecycle

**Files:**
- Create: `crates/lpc-vm/src/object.rs`
- Modify: `crates/lpc-vm/src/lib.rs`

Implement the LPC object model:
- **Master objects**: created by `compile_object()`, identified by file path
- **Clones**: created by `clone_object()`, identified as `path#N`
- **Light-weight objects**: created by `new_object()`, identified as `path#-1`,
  reference-counted
- Object table maps path → master, tracks all clones
- `destruct_object()` removes from table, marks destroyed
- `find_object()` looks up by path
- `this_object()`, `previous_object()` via call stack

Test object creation, cloning, destruction, and lookup.

**Step 1: Write failing tests**
**Step 2: Implement object table**
**Step 3: Run tests**
**Step 4: Commit**

---

### Task 12: Implement inheritance

**Files:**
- Modify: `crates/lpc-vm/src/object.rs`
- Modify: `crates/lpc-vm/src/vm.rs`

Implement LPC inheritance:
- `inherit "/path";` compiles parent, merges functions/variables
- Multiple inheritance with labels: `inherit label "/path";`
- `::function()` calls parent version
- `label::function()` calls specific parent
- `private inherit` restricts parent visibility
- Variable shadowing follows DGD rules

Build a dependency graph tracking which objects inherit which. This is the foundation
for the version tree and hot-reload.

Test with multi-level inheritance chains and diamond inheritance.

**Step 1: Write failing tests**
**Step 2: Implement inheritance resolution in compiler and VM**
**Step 3: Run tests**
**Step 4: Commit**

---

### Task 13: Implement core kfuns

**Files:**
- Create: `crates/lpc-vm/src/kfun/mod.rs`
- Create: `crates/lpc-vm/src/kfun/string.rs`
- Create: `crates/lpc-vm/src/kfun/array.rs`
- Create: `crates/lpc-vm/src/kfun/mapping.rs`
- Create: `crates/lpc-vm/src/kfun/math.rs`
- Create: `crates/lpc-vm/src/kfun/object.rs`
- Create: `crates/lpc-vm/src/kfun/type_ops.rs`
- Modify: `crates/lpc-vm/src/lib.rs`

Implement the kfun registry and core kfuns. The registry maps kfun names to Rust
function pointers. Kfuns are called via `CallKfun` opcode.

**Registry design:**

```rust
pub type KfunFn = fn(&mut VmContext, &[LpcValue]) -> Result<LpcValue, LpcError>;

pub struct KfunRegistry {
    kfuns: HashMap<String, (u16, KfunFn)>,  // name → (id, fn)
    by_id: Vec<KfunFn>,                      // id → fn
}

impl KfunRegistry {
    pub fn register(&mut self, name: &str, f: KfunFn) -> u16;
    pub fn call(&self, id: u16, ctx: &mut VmContext, args: &[LpcValue]) -> Result<LpcValue, LpcError>;
}
```

**Core kfuns to implement (pure computation, VM-internal):**

| Category | Kfuns |
|----------|-------|
| String | `strlen`, `explode`, `implode`, `sscanf`, `lower_case`, `upper_case` |
| Array | `allocate`, `sizeof`, `sort_array`, `filter`, `map` |
| Mapping | `map_indices`, `map_values`, `map_sizeof`, `mkmapping` |
| Math | `random`, `sqrt`, `sin`, `cos`, `tan`, `atan`, `atan2`, `exp`, `log`, `pow`, `floor`, `ceil`, `fabs`, `abs` |
| Object | `this_object`, `previous_object`, `object_name`, `function_object`, `typeof`, `sizeof` |
| Type | `typeof`, `instanceof` |
| Misc | `time`, `ctime`, `error`, `call_trace` |

Driver-service kfuns (`send_message`, `users`, file I/O, `compile_object`) are
implemented later in the adapter binary, registered via the extensible registry.

**Step 1: Write failing tests for each kfun category**
**Step 2: Implement kfun registry and core kfuns**
**Step 3: Run tests**
**Step 4: Commit per category**

---

### Task 14: Implement call_other and the arrow operator

**Files:**
- Modify: `crates/lpc-vm/src/vm.rs`

`call_other(obj, "method", args...)` and `obj->method(args)` are the core cross-object
communication mechanism. In the standalone VM they call between objects in the same
runtime. When integrated with the adapter, cross-area calls route through MOP.

```c
// These are equivalent:
call_other(sword, "get_description");
sword->get_description();
```

The VM must:
1. Resolve the target object (by ObjectRef or path string)
2. Look up the method in the target's compiled program
3. Create a new call frame on the target object
4. Execute and return the result
5. Set `previous_object()` correctly

**Step 1: Write failing tests**
**Step 2: Implement call_other in VM**
**Step 3: Run tests**
**Step 4: Commit**

---

### Task 15: Implement rlimits and atomic

**Files:**
- Modify: `crates/lpc-vm/src/vm.rs`

**rlimits:**
```c
rlimits(1000000; 100) {
    // code limited to 1M ticks and 100 stack depth
}
```
Push a resource limit frame. On entry, save current tick/stack limits, apply new ones.
On exit, restore. If ticks or stack depth exceeded, throw runtime error.

**atomic:**
```c
atomic void upgrade() {
    // all state changes roll back on error
}
```
On entry to an atomic function, snapshot the current state. On error, restore the
snapshot. No file I/O allowed inside atomic functions (error if attempted). Tick cost
is doubled inside atomic functions.

**Step 1: Write failing tests for rlimits and atomic rollback**
**Step 2: Implement in VM**
**Step 3: Run tests**
**Step 4: Commit**

---

### Task 16: Implement call_out (delayed execution)

**Files:**
- Create: `crates/lpc-vm/src/scheduler.rs`
- Modify: `crates/lpc-vm/src/lib.rs`

`call_out("function", delay, args...)` schedules a function to be called after `delay`
seconds. Returns a handle that can be cancelled with `remove_call_out(handle)`.

The scheduler maintains a priority queue of pending call_outs. The adapter's event loop
polls the scheduler and executes due call_outs.

**Step 1: Write failing tests for scheduling and cancellation**
**Step 2: Implement scheduler**
**Step 3: Run tests**
**Step 4: Commit**

---

### Task 17: Implement hot-reload (compile_object upgrade)

**Files:**
- Modify: `crates/lpc-vm/src/object.rs`
- Modify: `crates/lpc-vm/src/vm.rs`

When `compile_object(path)` is called on an already-compiled path:
1. Compile the new source
2. Replace the master object's program with the new version
3. Increment the version number
4. Walk the dependency graph: find all objects that inherit this one
5. For each dependent, call `upgraded(old_version)` if defined
6. Clones keep their state but get the new program

Test with a chain: A inherits B, B is recompiled, A gets upgraded().

**Step 1: Write failing tests**
**Step 2: Implement upgrade propagation**
**Step 3: Run tests**
**Step 4: Commit**

---

## Phase 5: LPC Adapter Binary

### Task 18: Create the adapter binary scaffold

**Files:**
- Create: `adapters/lpc/Cargo.toml`
- Create: `adapters/lpc/src/main.rs`
- Modify: `Cargo.toml` (workspace members)

```toml
# adapters/lpc/Cargo.toml
[package]
name = "mud-adapter-lpc"
version = "0.1.0"
edition = "2021"

[dependencies]
lpc-vm = { path = "../../crates/lpc-vm" }
mud-core = { path = "../../crates/mud-core" }
mud-mop = { path = "../../crates/mud-mop" }
tokio = { workspace = true }
serde = { workspace = true }
serde_yaml = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
clap = { workspace = true }
```

Entry point: parse `--socket` arg, connect to driver, send handshake, enter event loop.

**Step 1: Create scaffold**
**Step 2: Verify it compiles: `cargo build -p mud-adapter-lpc`**
**Step 3: Commit**

---

### Task 19: Implement MOP event loop and area loading

**Files:**
- Modify: `adapters/lpc/src/main.rs`
- Create: `adapters/lpc/src/area_loader.rs`
- Create: `adapters/lpc/src/session_handler.rs`

Handle the full MOP message lifecycle:
- `Configure` → store stdlib DB URL
- `LoadArea` → read `.c` files from path, compile in VM, send `AreaLoaded`/`AreaError`
- `ReloadArea` → recompile changed files, upgrade dependents
- `SessionStart/Input/End` → route to area's player object
- `Call` → call function on object, return `CallResult`/`CallError`
- `Ping` → reply `Pong`
- `CheckBuilderAccess` → delegate to stdlib if available
- `GetWebData` → delegate to area's web_data function

Register driver-service kfuns that route through MOP:
- `send_message` → `AdapterMessage::SendMessage`
- `users` → `DriverRequest { action: "list_users" }`
- `read_file`/`write_file` → `DriverRequest { action: "file_read"/"file_write" }`
- `compile_object` → compile locally + notify driver of version change

**Step 1: Write tests using mock MOP socket**
**Step 2: Implement event loop**
**Step 3: Run tests**
**Step 4: Commit**

---

### Task 20: Integrate with driver config and adapter manager

**Files:**
- Modify: `crates/mud-driver/src/config.rs`
- Modify: `crates/mud-driver/src/runtime/adapter_manager.rs`
- Modify: `crates/mud-driver/src/server.rs`

Add `LpcAdapterConfig` to the config struct. Update the adapter manager to spawn the
LPC adapter process. Update `language_for_area()` to recognize `language: lpc` in
`mud.yaml`.

Follow the existing JVM adapter integration pattern exactly.

**Step 1: Add config struct**
**Step 2: Add spawn logic to adapter_manager.rs**
**Step 3: Add language routing to server.rs**
**Step 4: Run existing tests to ensure nothing breaks: `cargo test -p mud-driver`**
**Step 5: Commit**

---

### Task 21: LPC area template

**Files:**
- Create: `adapters/lpc/templates/area/lpc/` (template files)

Create a default LPC area template that gets sent to the driver via
`set_area_template` on handshake:

```
templates/area/lpc/
├── mud.yaml              # language: lpc
├── rooms/
│   └── entrance.c        # Starting room
├── items/
│   └── torch.c           # Example item
├── npcs/
│   └── guide.c           # Example NPC
└── daemons/
    └── area_daemon.c     # Area initialization daemon
```

Template files use `{{namespace}}` and `{{area_name}}` placeholders (substituted by
driver on repo creation, same as Ruby/JVM templates).

**Step 1: Create template files**
**Step 2: Add template registration in adapter handshake**
**Step 3: Commit**

---

### Task 22: E2E tests for LPC adapter

**Files:**
- Create: `crates/mud-e2e/tests/lpc_adapter.rs`
- Modify: `crates/mud-e2e/src/harness.rs` (if needed)

Follow the pattern in `jvm_adapter.rs`:

```rust
#[tokio::test]
async fn lpc_adapter_connects_with_ruby() {
    // Start server with both adapters
    // Verify Ruby portal still works
    // Verify default area uses Ruby template
}

#[tokio::test]
async fn lpc_templates_registered() {
    // Verify lpc template appears in /api/repos/templates
}

#[tokio::test]
async fn lpc_template_area_creation() {
    // Create area from lpc template
    // Verify .c files exist, no Ruby files
    // Verify mud.yaml has language: lpc
}

#[tokio::test]
async fn lpc_area_loads_and_responds() {
    // Create LPC area, commit, wait for load
    // Connect session, send "look", verify output
}
```

**Step 1: Write the E2E tests**
**Step 2: Update harness if needed to build LPC adapter**
**Step 3: Run: `cargo test -p mud-e2e -- lpc`**
**Step 4: Commit**

---

## Phase 6: MOP Protocol Extensions

### Task 23: Add ReloadProgram and ProgramReloaded messages

**Files:**
- Modify: `crates/mud-mop/src/message.rs`

Add new variants to `DriverMessage` and `AdapterMessage`:

```rust
// DriverMessage additions:
#[serde(rename = "reload_program")]
ReloadProgram {
    area_id: AreaId,
    path: String,
    files: Vec<String>,  // changed file paths (relative to area root)
},

// AdapterMessage additions:
#[serde(rename = "program_reloaded")]
ProgramReloaded {
    area_id: AreaId,
    path: String,
    version: u64,
},

#[serde(rename = "program_reload_error")]
ProgramReloadError {
    area_id: AreaId,
    path: String,
    error: String,
},

#[serde(rename = "invalidate_cache")]
InvalidateCache {
    object_ids: Vec<ObjectId>,
},
```

**Step 1: Add message variants with serde attributes**
**Step 2: Write round-trip serialization tests**
**Step 3: Run: `cargo test -p mud-mop`**
**Step 4: Commit**

---

### Task 24: Add cache policy to CallResult

**Files:**
- Modify: `crates/mud-mop/src/message.rs`

Extend `CallResult` with an optional cache hint:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CachePolicy {
    #[serde(rename = "volatile")]
    Volatile,
    #[serde(rename = "cacheable")]
    Cacheable,
    #[serde(rename = "ttl")]
    Ttl(u64),  // seconds
}

// Updated CallResult:
#[serde(rename = "call_result")]
CallResult {
    request_id: u64,
    result: Value,
    #[serde(default)]
    cache: Option<CachePolicy>,
},
```

**Step 1: Add CachePolicy enum and update CallResult**
**Step 2: Write tests including backwards-compatible deserialization (missing cache field)**
**Step 3: Run: `cargo test -p mud-mop`**
**Step 4: Commit**

---

### Task 25: Implement diff-based reload in the driver

**Files:**
- Modify: `crates/mud-driver/src/server.rs`

When a git push/commit triggers a reload:
1. `git diff --name-only <old_commit>..<new_commit>` to get changed files
2. Group by file extension → determine target adapter
3. Send `ReloadProgram` (surgical) instead of `ReloadArea` (full)
4. Handle `ProgramReloaded` / `ProgramReloadError` responses
5. Broadcast `InvalidateCache` for affected object IDs

Retain `ReloadArea` as fallback for full reloads.

**Step 1: Write tests for diff grouping logic**
**Step 2: Implement diff-based reload**
**Step 3: Run tests**
**Step 4: Commit**

---

## Phase 7: Driver State Store and Object Broker

### Task 26: Implement the state store

**Files:**
- Create: `crates/mud-driver/src/runtime/state_store.rs`
- Modify: `crates/mud-driver/src/runtime/mod.rs`

The state store holds all object properties. In-memory with PostgreSQL backing for
persistence.

```rust
pub struct StateStore {
    objects: HashMap<ObjectId, ObjectState>,
}

pub struct ObjectState {
    pub program_path: String,
    pub language: String,
    pub version: u64,
    pub core_properties: HashMap<String, Value>,
    pub attached_properties: Vec<AttachedProperty>,
    pub location: Option<ObjectId>,
}

pub struct AttachedProperty {
    pub source: String,       // area that attached this
    pub key: String,
    pub value: Value,
}

impl StateStore {
    pub fn create_object(&mut self, id: ObjectId, program: &str, language: &str) -> &mut ObjectState;
    pub fn get(&self, id: ObjectId) -> Option<&ObjectState>;
    pub fn get_mut(&mut self, id: ObjectId) -> Option<&mut ObjectState>;
    pub fn set_property(&mut self, id: ObjectId, key: &str, value: Value);
    pub fn get_property(&self, id: ObjectId, key: &str) -> Option<&Value>;
    pub fn attach_property(&mut self, id: ObjectId, source: &str, key: &str, value: Value);
    pub fn remove_object(&mut self, id: ObjectId);
    pub fn upgrade_program(&mut self, id: ObjectId, new_version: u64);
}
```

**Step 1: Write tests for CRUD operations and property tagging**
**Step 2: Implement the state store**
**Step 3: Run tests**
**Step 4: Commit**

---

### Task 27: Implement the object broker

**Files:**
- Create: `crates/mud-driver/src/runtime/object_broker.rs`
- Modify: `crates/mud-driver/src/runtime/mod.rs`
- Modify: `crates/mud-driver/src/server.rs`

The object broker routes `call_other` across adapters:
1. Receive `Call` or `DriverRequest` with cross-area call
2. Look up target ObjectId in state store → determine language/adapter
3. If same adapter: route directly (adapter handles internally)
4. If different adapter: forward as `Call` message via MOP
5. Collect `CallResult`/`CallError` and return to caller
6. Apply cache policy if present

Also handles:
- Call result caching (check cache before forwarding)
- Cache invalidation on `InvalidateCache` messages

**Step 1: Write tests with mock adapters**
**Step 2: Implement broker**
**Step 3: Run tests**
**Step 4: Commit**

---

### Task 28: Implement the version tree

**Files:**
- Create: `crates/mud-driver/src/runtime/version_tree.rs`
- Modify: `crates/mud-driver/src/runtime/mod.rs`

The version tree tracks:
- Program paths → version numbers
- Dependency graph (who inherits/uses whom)
- Upgrade propagation

```rust
pub struct VersionTree {
    programs: HashMap<String, ProgramInfo>,
    dependents: HashMap<String, HashSet<String>>,  // path → who depends on it
}

pub struct ProgramInfo {
    pub path: String,
    pub version: u64,
    pub language: String,
    pub dependencies: Vec<String>,
}

impl VersionTree {
    pub fn register(&mut self, path: &str, language: &str, deps: Vec<String>);
    pub fn bump_version(&mut self, path: &str) -> u64;
    pub fn get_dependents(&self, path: &str) -> Vec<&str>;
    pub fn walk_dependents(&self, path: &str) -> Vec<&str>;  // transitive
}
```

**Step 1: Write tests for dependency graph and transitive walks**
**Step 2: Implement version tree**
**Step 3: Run tests**
**Step 4: Commit**

---

## Phase 8: Rust Dynamic Module System

### Task 29: Implement the .so module loader

**Files:**
- Create: `crates/lpc-vm/src/dynmod.rs` (or in the adapter crate)
- Modify: `crates/lpc-vm/src/lib.rs`

Load and unload `.so` dynamic libraries at runtime.

```rust
pub struct ModuleLoader {
    loaded: HashMap<String, LoadedModule>,
}

struct LoadedModule {
    path: String,
    library: libloading::Library,
    version: u64,
}

pub struct ModuleRegistrar {
    pub path: String,
    pub version: u64,
    pub dependencies: Vec<String>,
    pub kfuns: Vec<(String, KfunFn)>,
}

impl ModuleLoader {
    pub fn load(&mut self, so_path: &str) -> Result<ModuleRegistrar>;
    pub fn reload(&mut self, path: &str, so_path: &str) -> Result<ModuleRegistrar>;
    pub fn unload(&mut self, path: &str) -> Result<()>;
}
```

Each `.so` exports:
```rust
#[no_mangle]
pub extern "C" fn mud_module_init(registrar: &mut ModuleRegistrar);
```

Add `libloading` to dependencies.

**Step 1: Write tests with a test .so module**
**Step 2: Implement loader**
**Step 3: Run tests**
**Step 4: Commit**

---

### Task 30: Module build pipeline

**Files:**
- Create: `adapters/lpc/src/module_builder.rs`

When a `.rs` file changes in the stdlib:
1. Identify which module it belongs to (via `module.toml` or directory convention)
2. Run `cargo build --lib --manifest-path <module-cargo-toml>` to produce `.so`
3. Call `ModuleLoader::reload()` with the new `.so`
4. Fire upgrade notifications through the version tree

**Step 1: Write tests for build detection and triggering**
**Step 2: Implement builder**
**Step 3: Run tests**
**Step 4: Commit**

---

## Phase 9: Stdlib Foundation

### Task 31: Create LPC stdlib structure

**Files:**
- Create: `stdlib/lpc/sys/auto.c`
- Create: `stdlib/lpc/sys/driver.c`
- Create: `stdlib/lpc/std/base_object.c`
- Create: `stdlib/lpc/std/room.c`
- Create: `stdlib/lpc/std/item.c`
- Create: `stdlib/lpc/std/npc.c`
- Create: `stdlib/lpc/std/daemon.c`

Implement the auto object (inherited by all), driver object (VM↔driver interface),
and base game object classes with hook support.

The auto object provides:
- `dispatch_hooks()` — call registered hook handlers
- `register_hook()` / `unregister_hook()` — hook management
- Standard property accessors

The driver object implements DGD callbacks:
- `initialize()` — called on boot
- `path_read()` / `path_write()` — file access translation
- `compile_object()` — custom compilation hooks

Game objects (`room.c`, `item.c`, `npc.c`, `daemon.c`) inherit from `base_object.c`
and provide standard interfaces matching the Ruby/Kotlin stdlib.

**Step 1: Write auto.c and driver.c**
**Step 2: Write base_object.c with hook support**
**Step 3: Write room.c, item.c, npc.c, daemon.c**
**Step 4: Test with the LPC VM (compile and run basic programs using the stdlib)**
**Step 5: Commit**

---

### Task 32: Create LPC commands

**Files:**
- Create: `stdlib/lpc/cmd/look.c`
- Create: `stdlib/lpc/cmd/take.c`
- Create: `stdlib/lpc/cmd/drop.c`
- Create: `stdlib/lpc/cmd/say.c`
- Create: `stdlib/lpc/cmd/move.c`

Player commands that dispatch to room/item/npc methods. The command parser lives in
the player object and routes input to command files.

**Step 1: Write command implementations**
**Step 2: Write player.c and user.c with command routing**
**Step 3: Test end-to-end: connect → look → move → take**
**Step 4: Commit**

---

### Task 33: Implement parse_string kfun

**Files:**
- Create: `crates/lpc-vm/src/kfun/parse_string.rs`
- Modify: `crates/lpc-vm/src/kfun/mod.rs`

`parse_string` is DGD's context-free grammar parser — used for natural language
command parsing in MUD games. Implement the full spec:
- Token rules (regex → token type)
- Production rules (grammar → parse tree)
- LPC callbacks on rule matches
- Ambiguity handling (multiple parses, ranking)
- Bottom-up evaluation

This is a complex feature. Test with the standard MUD patterns:
- `"take the red sword from the chest"`
- `"go north"`, `"look at painting"`

**Step 1: Write failing tests with grammar definitions**
**Step 2: Implement the grammar parser**
**Step 3: Run tests**
**Step 4: Commit**

---

### Task 34: Implement save_object / restore_object

**Files:**
- Modify: `crates/lpc-vm/src/kfun/object.rs`

`save_object(file)` serializes non-private, non-static variables to a file.
`restore_object(file)` loads them back. In our architecture, this goes through
the driver's state store rather than direct file I/O.

The format is a text file with one variable per line: `name value\n`

**Step 1: Write tests for serialization format**
**Step 2: Implement via DriverServices trait**
**Step 3: Run tests**
**Step 4: Commit**

---

## Phase 10: Ruby and JVM Adapter Upgrades

### Task 35: Add cache policy support to Ruby adapter

**Files:**
- Modify: `adapters/ruby/lib/mud_adapter/stdlib/world/base.rb` (or equivalent)

Add `cacheable` and `volatile` method annotations to Ruby. Implement via
method metadata that the adapter includes in `CallResult` responses.

**Step 1: Add annotation DSL to Ruby base classes**
**Step 2: Include cache hint in MOP CallResult responses**
**Step 3: Test with mock driver**
**Step 4: Commit**

---

### Task 36: Add cache policy support to JVM adapter

**Files:**
- Modify: `adapters/jvm/stdlib/src/main/kotlin/mud/stdlib/annotations/`

Add `@Cacheable` and `@Volatile` annotations to Kotlin. The MOP client includes
cache hints in CallResult responses.

**Step 1: Add annotations**
**Step 2: Include cache hint in MOP responses**
**Step 3: Test**
**Step 4: Commit**

---

### Task 37: Add ReloadProgram support to Ruby and JVM adapters

**Files:**
- Modify: Ruby adapter MOP client
- Modify: JVM adapter MOP router

Handle `ReloadProgram` messages — reload only specified files instead of the
entire area. Ruby: re-evaluate specific `.rb` files. JVM: recompile specific classes.

**Step 1: Handle ReloadProgram in Ruby adapter**
**Step 2: Handle ReloadProgram in JVM adapter**
**Step 3: E2E test: modify one file, verify surgical reload**
**Step 4: Commit**

---

## Summary

| Phase | Tasks | Focus |
|-------|-------|-------|
| 1 | 1-4 | LPC VM lexer and preprocessor |
| 2 | 5-7 | LPC VM parser (AST, expressions, statements) |
| 3 | 8-10 | Compiler and bytecode VM |
| 4 | 11-17 | Object model, inheritance, kfuns, hot-reload |
| 5 | 18-22 | LPC adapter binary and E2E tests |
| 6 | 23-25 | MOP protocol extensions and diff-based reload |
| 7 | 26-28 | Driver state store, object broker, version tree |
| 8 | 29-30 | Rust dynamic module system |
| 9 | 31-34 | LPC stdlib (game objects, commands, persistence) |
| 10 | 35-37 | Ruby and JVM adapter upgrades |

**Dependencies:**
- Phases 1-4 are independent (pure `lpc-vm` crate, no driver integration)
- Phase 5 depends on Phase 4 (needs working VM)
- Phase 6 can start in parallel with Phase 5 (MOP changes are independent)
- Phase 7 depends on Phase 6 (uses new MOP messages)
- Phase 8 can start after Phase 4 (`.so` loading is independent of MOP)
- Phase 9 depends on Phases 4 + 5 (needs VM + adapter)
- Phase 10 depends on Phase 6 (needs new MOP messages)
