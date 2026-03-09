/// Source location span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: u32,
    pub col: u32,
}

impl Span {
    pub fn new(start: usize, end: usize, line: u32, col: u32) -> Self {
        Self {
            start,
            end,
            line,
            col,
        }
    }

    pub fn dummy() -> Self {
        Self {
            start: 0,
            end: 0,
            line: 0,
            col: 0,
        }
    }
}

/// A single token produced by the lexer.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    pub text: String,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span, text: impl Into<String>) -> Self {
        Self {
            kind,
            span,
            text: text.into(),
        }
    }
}

/// All token kinds for DGD-compatible LPC.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // -- Literals --
    IntLiteral(i64),
    FloatLiteral(f64),
    StringLiteral(String),
    CharLiteral(char),

    // -- Identifier --
    Identifier(String),

    // -- Keywords --
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
    String_,
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

    // -- Single-char operators --
    Plus,       // +
    Minus,      // -
    Star,       // *
    Slash,      // /
    Percent,    // %
    Ampersand,  // &
    Pipe,       // |
    Caret,      // ^
    Tilde,      // ~
    Bang,       // !
    Assign,     // =
    Less,       // <
    Greater,    // >
    Dot,        // .
    Question,   // ?
    Colon,      // :

    // -- Multi-char operators --
    PlusPlus,       // ++
    MinusMinus,     // --
    Arrow,          // ->
    ColonColon,     // ::
    DotDot,         // ..
    Ellipsis,       // ...
    EqEq,           // ==
    NotEq,          // !=
    LessEq,         // <=
    GreaterEq,      // >=
    AndAnd,         // &&
    OrOr,           // ||
    ShiftLeft,      // <<
    ShiftRight,     // >>
    PlusAssign,     // +=
    MinusAssign,    // -=
    StarAssign,     // *=
    SlashAssign,    // /=
    PercentAssign,  // %=
    AmpAssign,      // &=
    PipeAssign,     // |=
    CaretAssign,    // ^=
    ShlAssign,      // <<=
    ShrAssign,      // >>=

    // -- Delimiters --
    LParen,         // (
    RParen,         // )
    LBrace,         // {
    RBrace,         // }
    LBracket,       // [
    RBracket,       // ]
    MappingOpen,    // ([
    MappingClose,   // ])
    Semicolon,      // ;
    Comma,          // ,

    // -- End of file --
    Eof,
}

/// Look up whether the given identifier is a keyword and return the corresponding TokenKind.
pub fn keyword_lookup(ident: &str) -> Option<TokenKind> {
    match ident {
        "if" => Some(TokenKind::If),
        "else" => Some(TokenKind::Else),
        "while" => Some(TokenKind::While),
        "do" => Some(TokenKind::Do),
        "for" => Some(TokenKind::For),
        "switch" => Some(TokenKind::Switch),
        "case" => Some(TokenKind::Case),
        "default" => Some(TokenKind::Default),
        "break" => Some(TokenKind::Break),
        "continue" => Some(TokenKind::Continue),
        "return" => Some(TokenKind::Return),
        "inherit" => Some(TokenKind::Inherit),
        "private" => Some(TokenKind::Private),
        "static" => Some(TokenKind::Static),
        "nomask" => Some(TokenKind::Nomask),
        "atomic" => Some(TokenKind::Atomic),
        "varargs" => Some(TokenKind::Varargs),
        "int" => Some(TokenKind::Int),
        "float" => Some(TokenKind::Float),
        "string" => Some(TokenKind::String_),
        "object" => Some(TokenKind::Object),
        "mapping" => Some(TokenKind::Mapping),
        "mixed" => Some(TokenKind::Mixed),
        "void" => Some(TokenKind::Void),
        "nil" => Some(TokenKind::Nil),
        "rlimits" => Some(TokenKind::Rlimits),
        "catch" => Some(TokenKind::Catch),
        "sizeof" => Some(TokenKind::Sizeof),
        "typeof" => Some(TokenKind::Typeof),
        "new" => Some(TokenKind::New),
        _ => None,
    }
}
