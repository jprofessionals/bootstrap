use lpc_vm::lexer::scanner::{LexError, Scanner};
use lpc_vm::lexer::token::TokenKind;

/// Helper: scan all tokens from source (excludes Eof).
fn scan(src: &str) -> Result<Vec<TokenKind>, LexError> {
    let mut scanner = Scanner::new(src);
    let tokens = scanner.scan_all()?;
    Ok(tokens.into_iter().map(|t| t.kind).collect())
}

/// Helper: scan all tokens, dropping the trailing Eof.
fn scan_no_eof(src: &str) -> Result<Vec<TokenKind>, LexError> {
    let mut kinds = scan(src)?;
    if matches!(kinds.last(), Some(TokenKind::Eof)) {
        kinds.pop();
    }
    Ok(kinds)
}

// =========================================================================
// Empty input
// =========================================================================

#[test]
fn empty_input_produces_only_eof() {
    let kinds = scan("").unwrap();
    assert_eq!(kinds, vec![TokenKind::Eof]);
}

// =========================================================================
// Integer literals
// =========================================================================

#[test]
fn decimal_integer() {
    let kinds = scan_no_eof("42").unwrap();
    assert_eq!(kinds, vec![TokenKind::IntLiteral(42)]);
}

#[test]
fn hex_integer() {
    let kinds = scan_no_eof("0x1F").unwrap();
    assert_eq!(kinds, vec![TokenKind::IntLiteral(0x1F)]);
}

#[test]
fn hex_integer_uppercase() {
    let kinds = scan_no_eof("0X1A").unwrap();
    assert_eq!(kinds, vec![TokenKind::IntLiteral(0x1A)]);
}

#[test]
fn octal_integer() {
    let kinds = scan_no_eof("077").unwrap();
    assert_eq!(kinds, vec![TokenKind::IntLiteral(0o77)]);
}

#[test]
fn zero_literal() {
    let kinds = scan_no_eof("0").unwrap();
    assert_eq!(kinds, vec![TokenKind::IntLiteral(0)]);
}

// =========================================================================
// Float literals
// =========================================================================

#[test]
fn float_basic() {
    let kinds = scan_no_eof("3.14").unwrap();
    assert_eq!(kinds, vec![TokenKind::FloatLiteral(3.14)]);
}

#[test]
fn float_leading_dot() {
    let kinds = scan_no_eof(".5").unwrap();
    assert_eq!(kinds, vec![TokenKind::FloatLiteral(0.5)]);
}

#[test]
fn float_scientific() {
    let kinds = scan_no_eof("1e10").unwrap();
    assert_eq!(kinds, vec![TokenKind::FloatLiteral(1e10)]);
}

#[test]
fn float_scientific_with_neg_exponent() {
    let kinds = scan_no_eof("1.5e-3").unwrap();
    assert_eq!(kinds, vec![TokenKind::FloatLiteral(1.5e-3)]);
}

// =========================================================================
// String literals
// =========================================================================

#[test]
fn string_basic() {
    let kinds = scan_no_eof(r#""hello""#).unwrap();
    assert_eq!(kinds, vec![TokenKind::StringLiteral("hello".to_string())]);
}

#[test]
fn string_with_newline_escape() {
    let kinds = scan_no_eof(r#""with\nnewline""#).unwrap();
    assert_eq!(
        kinds,
        vec![TokenKind::StringLiteral("with\nnewline".to_string())]
    );
}

#[test]
fn string_with_escaped_quote() {
    let kinds = scan_no_eof(r#""with\"quote""#).unwrap();
    assert_eq!(
        kinds,
        vec![TokenKind::StringLiteral("with\"quote".to_string())]
    );
}

#[test]
fn string_with_tab_escape() {
    let kinds = scan_no_eof(r#""tab\there""#).unwrap();
    assert_eq!(
        kinds,
        vec![TokenKind::StringLiteral("tab\there".to_string())]
    );
}

// =========================================================================
// Char literals
// =========================================================================

#[test]
fn char_basic() {
    let kinds = scan_no_eof("'a'").unwrap();
    assert_eq!(kinds, vec![TokenKind::CharLiteral('a')]);
}

#[test]
fn char_escaped_newline() {
    let kinds = scan_no_eof(r"'\n'").unwrap();
    assert_eq!(kinds, vec![TokenKind::CharLiteral('\n')]);
}

#[test]
fn char_escaped_tab() {
    let kinds = scan_no_eof(r"'\t'").unwrap();
    assert_eq!(kinds, vec![TokenKind::CharLiteral('\t')]);
}

// =========================================================================
// Keywords
// =========================================================================

#[test]
fn all_keywords() {
    let tests: Vec<(&str, TokenKind)> = vec![
        ("if", TokenKind::If),
        ("else", TokenKind::Else),
        ("while", TokenKind::While),
        ("do", TokenKind::Do),
        ("for", TokenKind::For),
        ("switch", TokenKind::Switch),
        ("case", TokenKind::Case),
        ("default", TokenKind::Default),
        ("break", TokenKind::Break),
        ("continue", TokenKind::Continue),
        ("return", TokenKind::Return),
        ("inherit", TokenKind::Inherit),
        ("private", TokenKind::Private),
        ("static", TokenKind::Static),
        ("nomask", TokenKind::Nomask),
        ("atomic", TokenKind::Atomic),
        ("varargs", TokenKind::Varargs),
        ("int", TokenKind::Int),
        ("float", TokenKind::Float),
        ("string", TokenKind::String_),
        ("object", TokenKind::Object),
        ("mapping", TokenKind::Mapping),
        ("mixed", TokenKind::Mixed),
        ("void", TokenKind::Void),
        ("nil", TokenKind::Nil),
        ("rlimits", TokenKind::Rlimits),
        ("catch", TokenKind::Catch),
        ("sizeof", TokenKind::Sizeof),
        ("typeof", TokenKind::Typeof),
        ("new", TokenKind::New),
    ];
    for (src, expected) in tests {
        let kinds = scan_no_eof(src).unwrap();
        assert_eq!(kinds, vec![expected], "keyword: {}", src);
    }
}

// =========================================================================
// Identifiers
// =========================================================================

#[test]
fn identifier_simple() {
    let kinds = scan_no_eof("foo").unwrap();
    assert_eq!(kinds, vec![TokenKind::Identifier("foo".into())]);
}

#[test]
fn identifier_with_underscore_prefix() {
    let kinds = scan_no_eof("_bar").unwrap();
    assert_eq!(kinds, vec![TokenKind::Identifier("_bar".into())]);
}

#[test]
fn identifier_with_digits() {
    let kinds = scan_no_eof("x123").unwrap();
    assert_eq!(kinds, vec![TokenKind::Identifier("x123".into())]);
}

// =========================================================================
// Operators
// =========================================================================

#[test]
fn single_char_operators() {
    let tests: Vec<(&str, TokenKind)> = vec![
        ("+", TokenKind::Plus),
        ("-", TokenKind::Minus),
        ("*", TokenKind::Star),
        ("/", TokenKind::Slash),
        ("%", TokenKind::Percent),
        ("&", TokenKind::Ampersand),
        ("|", TokenKind::Pipe),
        ("^", TokenKind::Caret),
        ("~", TokenKind::Tilde),
        ("!", TokenKind::Bang),
        ("=", TokenKind::Assign),
        ("<", TokenKind::Less),
        (">", TokenKind::Greater),
        (".", TokenKind::Dot),
        ("?", TokenKind::Question),
        (":", TokenKind::Colon),
    ];
    for (src, expected) in tests {
        let kinds = scan_no_eof(src).unwrap();
        assert_eq!(kinds, vec![expected], "operator: {}", src);
    }
}

#[test]
fn multi_char_operators() {
    let tests: Vec<(&str, TokenKind)> = vec![
        ("++", TokenKind::PlusPlus),
        ("--", TokenKind::MinusMinus),
        ("->", TokenKind::Arrow),
        ("::", TokenKind::ColonColon),
        ("..", TokenKind::DotDot),
        ("...", TokenKind::Ellipsis),
        ("==", TokenKind::EqEq),
        ("!=", TokenKind::NotEq),
        ("<=", TokenKind::LessEq),
        (">=", TokenKind::GreaterEq),
        ("&&", TokenKind::AndAnd),
        ("||", TokenKind::OrOr),
        ("<<", TokenKind::ShiftLeft),
        (">>", TokenKind::ShiftRight),
        ("+=", TokenKind::PlusAssign),
        ("-=", TokenKind::MinusAssign),
        ("*=", TokenKind::StarAssign),
        ("/=", TokenKind::SlashAssign),
        ("%=", TokenKind::PercentAssign),
        ("&=", TokenKind::AmpAssign),
        ("|=", TokenKind::PipeAssign),
        ("^=", TokenKind::CaretAssign),
        ("<<=", TokenKind::ShlAssign),
        (">>=", TokenKind::ShrAssign),
    ];
    for (src, expected) in tests {
        let kinds = scan_no_eof(src).unwrap();
        assert_eq!(kinds, vec![expected], "operator: {}", src);
    }
}

// =========================================================================
// Delimiters
// =========================================================================

#[test]
fn delimiters() {
    let tests: Vec<(&str, TokenKind)> = vec![
        ("(", TokenKind::LParen),
        (")", TokenKind::RParen),
        ("{", TokenKind::LBrace),
        ("}", TokenKind::RBrace),
        ("[", TokenKind::LBracket),
        ("]", TokenKind::RBracket),
        (";", TokenKind::Semicolon),
        (",", TokenKind::Comma),
    ];
    for (src, expected) in tests {
        let kinds = scan_no_eof(src).unwrap();
        assert_eq!(kinds, vec![expected], "delimiter: {}", src);
    }
}

#[test]
fn mapping_open_close() {
    let kinds = scan_no_eof("([").unwrap();
    assert_eq!(kinds, vec![TokenKind::MappingOpen]);
    let kinds = scan_no_eof("])").unwrap();
    assert_eq!(kinds, vec![TokenKind::MappingClose]);
}

// =========================================================================
// Comments
// =========================================================================

#[test]
fn line_comment_skipped() {
    let kinds = scan_no_eof("42 // this is a comment\n7").unwrap();
    assert_eq!(
        kinds,
        vec![TokenKind::IntLiteral(42), TokenKind::IntLiteral(7)]
    );
}

#[test]
fn block_comment_skipped() {
    let kinds = scan_no_eof("42 /* block */ 7").unwrap();
    assert_eq!(
        kinds,
        vec![TokenKind::IntLiteral(42), TokenKind::IntLiteral(7)]
    );
}

#[test]
fn multiline_block_comment() {
    let kinds = scan_no_eof("1 /* multi\nline\ncomment */ 2").unwrap();
    assert_eq!(
        kinds,
        vec![TokenKind::IntLiteral(1), TokenKind::IntLiteral(2)]
    );
}

// =========================================================================
// Mapping literal tokens
// =========================================================================

#[test]
fn mapping_literal_tokens() {
    let kinds = scan_no_eof(r#"([ "key" : 1 ])"#).unwrap();
    assert_eq!(
        kinds,
        vec![
            TokenKind::MappingOpen,
            TokenKind::StringLiteral("key".into()),
            TokenKind::Colon,
            TokenKind::IntLiteral(1),
            TokenKind::MappingClose,
        ]
    );
}

// =========================================================================
// LPC function signature
// =========================================================================

#[test]
fn void_create_function_signature() {
    let kinds = scan_no_eof("void create()").unwrap();
    assert_eq!(
        kinds,
        vec![
            TokenKind::Void,
            TokenKind::Identifier("create".into()),
            TokenKind::LParen,
            TokenKind::RParen,
        ]
    );
}

// =========================================================================
// Inherit statement
// =========================================================================

#[test]
fn inherit_statement_tokens() {
    let kinds = scan_no_eof(r#"inherit "/std/room";"#).unwrap();
    assert_eq!(
        kinds,
        vec![
            TokenKind::Inherit,
            TokenKind::StringLiteral("/std/room".into()),
            TokenKind::Semicolon,
        ]
    );
}

// =========================================================================
// Span tracking
// =========================================================================

#[test]
fn span_tracks_line_and_column() {
    let mut scanner = Scanner::new("abc\ndef");
    let tokens = scanner.scan_all().unwrap();
    // "abc" at line 1, col 1
    assert_eq!(tokens[0].span.line, 1);
    assert_eq!(tokens[0].span.col, 1);
    // "def" at line 2, col 1
    assert_eq!(tokens[1].span.line, 2);
    assert_eq!(tokens[1].span.col, 1);
}

#[test]
fn span_column_increments() {
    let mut scanner = Scanner::new("a b");
    let tokens = scanner.scan_all().unwrap();
    // "a" at col 1
    assert_eq!(tokens[0].span.col, 1);
    // "b" at col 3
    assert_eq!(tokens[1].span.col, 3);
}

// =========================================================================
// Error cases
// =========================================================================

#[test]
fn unterminated_string_error() {
    let result = scan(r#""unterminated"#);
    assert!(result.is_err());
    match result.unwrap_err() {
        LexError::UnterminatedString { .. } => {}
        other => panic!("expected UnterminatedString, got: {:?}", other),
    }
}

#[test]
fn unterminated_block_comment_error() {
    let result = scan("/* never closed");
    assert!(result.is_err());
    match result.unwrap_err() {
        LexError::UnterminatedComment { .. } => {}
        other => panic!("expected UnterminatedComment, got: {:?}", other),
    }
}

#[test]
fn unexpected_character_error() {
    let result = scan("@");
    assert!(result.is_err());
    match result.unwrap_err() {
        LexError::UnexpectedChar { ch, .. } => assert_eq!(ch, '@'),
        other => panic!("expected UnexpectedChar, got: {:?}", other),
    }
}

// =========================================================================
// Whitespace-only input
// =========================================================================

#[test]
fn whitespace_only_produces_eof() {
    let kinds = scan("   \n\t\n  ").unwrap();
    assert_eq!(kinds, vec![TokenKind::Eof]);
}

// =========================================================================
// Multiple tokens in sequence
// =========================================================================

#[test]
fn mixed_tokens_sequence() {
    let kinds = scan_no_eof("int x = 42;").unwrap();
    assert_eq!(
        kinds,
        vec![
            TokenKind::Int,
            TokenKind::Identifier("x".into()),
            TokenKind::Assign,
            TokenKind::IntLiteral(42),
            TokenKind::Semicolon,
        ]
    );
}

#[test]
fn string_empty() {
    let kinds = scan_no_eof(r#""""#).unwrap();
    assert_eq!(kinds, vec![TokenKind::StringLiteral("".into())]);
}
