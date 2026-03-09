use crate::lexer::Span;

/// Top-level program: a sequence of inherit declarations and top-level declarations.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub inherits: Vec<InheritDecl>,
    pub declarations: Vec<Declaration>,
}

/// An `inherit` directive.
#[derive(Debug, Clone, PartialEq)]
pub struct InheritDecl {
    pub label: Option<String>,
    pub path: String,
    pub access: AccessModifier,
    pub span: Span,
}

/// Access modifier for inherit and declarations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessModifier {
    Public,
    Private,
}

/// A top-level declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum Declaration {
    Function(FunctionDecl),
    Variable(VarDecl),
}

/// Function declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDecl {
    pub modifiers: Vec<Modifier>,
    pub return_type: TypeExpr,
    pub name: String,
    pub params: Vec<Param>,
    pub body: Block,
    pub span: Span,
}

/// Declaration modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    Private,
    Static,
    Nomask,
    Atomic,
    Varargs,
}

/// A type expression, with optional array depth (e.g., `int *` is depth 1, `string **` is depth 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeExpr {
    pub base: BaseType,
    pub array_depth: u32,
}

/// Base types in LPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseType {
    Int,
    Float,
    String,
    Object,
    Mapping,
    Mixed,
    Void,
}

/// Function parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub type_expr: TypeExpr,
    pub name: String,
    pub varargs: bool,
}

/// Variable declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    pub modifiers: Vec<Modifier>,
    pub type_expr: TypeExpr,
    pub name: String,
    pub initializer: Option<Expr>,
    pub span: Span,
}

/// A block is a list of statements.
pub type Block = Vec<Stmt>;

/// Statements.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// Expression statement.
    Expr(ExprStmt),
    /// `if (cond) then_branch [else else_branch]`
    If(IfStmt),
    /// `while (cond) body`
    While(WhileStmt),
    /// `do body while (cond);`
    DoWhile(DoWhileStmt),
    /// `for (init; cond; step) body`
    For(ForStmt),
    /// `switch (expr) { ... }`
    Switch(SwitchStmt),
    /// `return [expr];`
    Return(ReturnStmt),
    /// `break;`
    Break(Span),
    /// `continue;`
    Continue(Span),
    /// `{ ... }`
    Block(BlockStmt),
    /// `rlimits (stack; ticks) { ... }`
    Rlimits(RlimitsStmt),
    /// `catch { ... } [: expr]`
    Catch(CatchStmt),
    /// Local variable declaration inside a function body.
    VarDecl(VarDecl),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExprStmt {
    pub expr: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfStmt {
    pub condition: Expr,
    pub then_branch: Box<Stmt>,
    pub else_branch: Option<Box<Stmt>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhileStmt {
    pub condition: Expr,
    pub body: Box<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DoWhileStmt {
    pub body: Box<Stmt>,
    pub condition: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForStmt {
    pub init: Option<Box<Expr>>,
    pub condition: Option<Box<Expr>>,
    pub step: Option<Box<Expr>>,
    pub body: Box<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SwitchStmt {
    pub expr: Expr,
    pub cases: Vec<SwitchCase>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SwitchCase {
    pub label: CaseLabel,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CaseLabel {
    /// `case expr:`
    Expr(Expr),
    /// `case expr .. expr:` (range case)
    Range(Expr, Expr),
    /// `default:`
    Default,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReturnStmt {
    pub value: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockStmt {
    pub stmts: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RlimitsStmt {
    pub stack: Expr,
    pub ticks: Expr,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CatchStmt {
    pub body: Block,
    pub handler: Option<Expr>,
    pub span: Span,
}

/// Expressions.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Integer literal.
    IntLiteral(i64, Span),
    /// Float literal.
    FloatLiteral(f64, Span),
    /// String literal.
    StringLiteral(String, Span),
    /// Character literal.
    CharLiteral(char, Span),
    /// `nil` literal.
    NilLiteral(Span),
    /// Variable or function reference.
    Identifier(String, Span),
    /// Binary operation: `left op right`.
    Binary(BinaryExpr),
    /// Unary prefix operation: `op expr`.
    Unary(UnaryExpr),
    /// Post-increment: `expr++`.
    PostIncrement(Box<Expr>, Span),
    /// Post-decrement: `expr--`.
    PostDecrement(Box<Expr>, Span),
    /// Assignment: `target op value`.
    Assign(AssignExpr),
    /// Ternary: `cond ? then : else`.
    Ternary(TernaryExpr),
    /// Function call: `func(args...)`.
    Call(CallExpr),
    /// Call other: `obj->func(args...)`.
    CallOther(CallOtherExpr),
    /// Parent (inherited) call: `::func(args...)` or `label::func(args...)`.
    ParentCall(ParentCallExpr),
    /// Index: `expr[index]`.
    Index(IndexExpr),
    /// Range: `expr[start..end]`.
    Range(RangeExpr),
    /// Array literal: `({ ... })`.
    ArrayLiteral(Vec<Expr>, Span),
    /// Mapping literal: `([ key: value, ... ])`.
    MappingLiteral(Vec<(Expr, Expr)>, Span),
    /// Type cast: `(type) expr`.
    Cast(CastExpr),
    /// `sizeof(expr)`.
    Sizeof(Box<Expr>, Span),
    /// `typeof(expr)`.
    Typeof(Box<Expr>, Span),
    /// `new_object(path)` or `clone_object(path)`.
    NewObject(Box<Expr>, Span),
    /// `catch(expr)` as expression form.
    CatchExpr(Box<Expr>, Span),
    /// Comma expression: `expr, expr`.
    Comma(Vec<Expr>, Span),
}

#[derive(Debug, Clone, PartialEq)]
pub struct BinaryExpr {
    pub left: Box<Expr>,
    pub op: BinaryOp,
    pub right: Box<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnaryExpr {
    pub op: UnaryOp,
    pub expr: Box<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssignExpr {
    pub target: Box<Expr>,
    pub op: AssignOp,
    pub value: Box<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TernaryExpr {
    pub condition: Box<Expr>,
    pub then_expr: Box<Expr>,
    pub else_expr: Box<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallExpr {
    pub function: Box<Expr>,
    pub args: Vec<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallOtherExpr {
    pub object: Box<Expr>,
    pub method: String,
    pub args: Vec<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParentCallExpr {
    pub label: Option<String>,
    pub function: String,
    pub args: Vec<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndexExpr {
    pub object: Box<Expr>,
    pub index: Box<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RangeExpr {
    pub object: Box<Expr>,
    pub start: Option<Box<Expr>>,
    pub end: Option<Box<Expr>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CastExpr {
    pub type_expr: TypeExpr,
    pub expr: Box<Expr>,
    pub span: Span,
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    NotEq,
    Less,
    LessEq,
    Greater,
    GreaterEq,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
    PreIncrement,
    PreDecrement,
}

/// Assignment operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Assign,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    ModAssign,
    AndAssign,
    OrAssign,
    XorAssign,
    ShlAssign,
    ShrAssign,
}
