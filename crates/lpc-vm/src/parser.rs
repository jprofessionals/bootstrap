use crate::ast::*;
use crate::lexer::{Span, Token, TokenKind};

/// Parse errors with location information.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ParseError {
    #[error("unexpected token {found:?}, expected {expected} at line {line}, col {col}")]
    UnexpectedToken {
        found: TokenKind,
        expected: String,
        line: u32,
        col: u32,
    },

    #[error("unexpected end of input, expected {expected}")]
    UnexpectedEof { expected: String },

    #[error("{message} at line {line}, col {col}")]
    General {
        message: String,
        line: u32,
        col: u32,
    },
}

/// Recursive-descent parser with Pratt precedence for LPC.
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

/// Binding power for Pratt parsing.  Higher = tighter binding.
/// We use even numbers so left-associative ops use (lbp, lbp+1) and
/// right-associative ops use (lbp, lbp).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Bp(u8);

const _BP_NONE: Bp = Bp(0);
const BP_COMMA: Bp = Bp(2);
const BP_ASSIGN: Bp = Bp(4);
const BP_TERNARY: Bp = Bp(6);
const BP_OR: Bp = Bp(8);
const BP_AND: Bp = Bp(10);
const BP_BIT_OR: Bp = Bp(12);
const BP_BIT_XOR: Bp = Bp(14);
const BP_BIT_AND: Bp = Bp(16);
const BP_EQUALITY: Bp = Bp(18);
const BP_RELATIONAL: Bp = Bp(20);
const BP_SHIFT: Bp = Bp(22);
const BP_ADDITIVE: Bp = Bp(24);
const BP_MULTIPLICATIVE: Bp = Bp(26);
const BP_UNARY: Bp = Bp(28);
const _BP_POSTFIX: Bp = Bp(30);

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    pub fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut inherits = Vec::new();
        let mut declarations = Vec::new();

        while !self.is_at_end() {
            if self.check(&TokenKind::Inherit) {
                inherits.push(self.parse_inherit()?);
            } else if self.check(&TokenKind::Private) && self.check_ahead(1, &TokenKind::Inherit) {
                inherits.push(self.parse_inherit()?);
            } else {
                declarations.push(self.parse_declaration()?);
            }
        }

        Ok(Program {
            inherits,
            declarations,
        })
    }

    // ---- Inherit ----

    fn parse_inherit(&mut self) -> Result<InheritDecl, ParseError> {
        let span_start = self.current_span();

        // Optional access modifier
        let access = if self.check(&TokenKind::Private) {
            self.advance();
            AccessModifier::Private
        } else {
            AccessModifier::Public
        };

        self.expect(&TokenKind::Inherit)?;

        // Optional label (identifier before the string)
        let mut label = None;
        if self.check_identifier() && self.check_ahead_string_literal(1) {
            label = Some(self.expect_identifier()?);
        }

        let path = self.expect_string_literal()?;
        self.expect(&TokenKind::Semicolon)?;

        Ok(InheritDecl {
            label,
            path,
            access,
            span: span_start,
        })
    }

    // ---- Top-level declarations ----

    fn parse_declaration(&mut self) -> Result<Declaration, ParseError> {
        // Parse modifiers
        let modifiers = self.parse_modifiers();

        // Parse type
        let type_expr = self.parse_type_expr()?;

        // Must be followed by identifier
        let name = self.expect_identifier()?;

        // Function: name(...)
        if self.check(&TokenKind::LParen) {
            return self.parse_function_decl(modifiers, type_expr, name);
        }

        // Variable: name [= expr] ;
        self.parse_var_decl_rest(modifiers, type_expr, name)
    }

    fn parse_modifiers(&mut self) -> Vec<Modifier> {
        let mut modifiers = Vec::new();
        loop {
            let m = match self.current_kind() {
                TokenKind::Private => Modifier::Private,
                TokenKind::Static => Modifier::Static,
                TokenKind::Nomask => Modifier::Nomask,
                TokenKind::Atomic => Modifier::Atomic,
                TokenKind::Varargs => Modifier::Varargs,
                _ => break,
            };
            // Only treat as modifier if followed by something that looks like
            // a type or another modifier (to distinguish `static` as modifier vs type keyword).
            // But all these keywords are *only* modifiers in LPC, so always consume.
            modifiers.push(m);
            self.advance();
        }
        modifiers
    }

    fn parse_function_decl(
        &mut self,
        modifiers: Vec<Modifier>,
        return_type: TypeExpr,
        name: String,
    ) -> Result<Declaration, ParseError> {
        let span = self.current_span();
        let params = self.parse_params()?;
        let body = self.parse_block()?;

        Ok(Declaration::Function(FunctionDecl {
            modifiers,
            return_type,
            name,
            params,
            body,
            span,
        }))
    }

    fn parse_params(&mut self) -> Result<Vec<Param>, ParseError> {
        self.expect(&TokenKind::LParen)?;
        let mut params = Vec::new();

        if !self.check(&TokenKind::RParen) {
            loop {
                let varargs = if self.check(&TokenKind::Varargs) {
                    self.advance();
                    true
                } else {
                    false
                };

                let type_expr = self.parse_type_expr()?;
                let name = self.expect_identifier()?;

                params.push(Param {
                    type_expr,
                    name,
                    varargs,
                });

                if !self.check(&TokenKind::Comma) {
                    break;
                }
                self.advance(); // consume comma
            }
        }

        self.expect(&TokenKind::RParen)?;
        Ok(params)
    }

    fn parse_var_decl_rest(
        &mut self,
        modifiers: Vec<Modifier>,
        type_expr: TypeExpr,
        name: String,
    ) -> Result<Declaration, ParseError> {
        let span = self.current_span();
        let initializer = if self.check(&TokenKind::Assign) {
            self.advance();
            Some(self.parse_assignment_expr()?)
        } else {
            None
        };

        self.expect(&TokenKind::Semicolon)?;

        Ok(Declaration::Variable(VarDecl {
            modifiers,
            type_expr,
            name,
            initializer,
            span,
        }))
    }

    // ---- Types ----

    fn parse_type_expr(&mut self) -> Result<TypeExpr, ParseError> {
        let base = self.parse_base_type()?;
        let mut array_depth = 0u32;

        while self.check(&TokenKind::Star) {
            self.advance();
            array_depth += 1;
        }

        Ok(TypeExpr { base, array_depth })
    }

    fn parse_base_type(&mut self) -> Result<BaseType, ParseError> {
        let kind = self.current_kind().clone();
        match kind {
            TokenKind::Int => {
                self.advance();
                Ok(BaseType::Int)
            }
            TokenKind::Float => {
                self.advance();
                Ok(BaseType::Float)
            }
            TokenKind::String_ => {
                self.advance();
                Ok(BaseType::String)
            }
            TokenKind::Object => {
                self.advance();
                Ok(BaseType::Object)
            }
            TokenKind::Mapping => {
                self.advance();
                Ok(BaseType::Mapping)
            }
            TokenKind::Mixed => {
                self.advance();
                Ok(BaseType::Mixed)
            }
            TokenKind::Void => {
                self.advance();
                Ok(BaseType::Void)
            }
            _ => {
                let span = self.current_span();
                Err(ParseError::UnexpectedToken {
                    found: kind,
                    expected: "type".to_string(),
                    line: span.line,
                    col: span.col,
                })
            }
        }
    }

    /// Check if the current token is a base type keyword.
    fn is_type_keyword(&self) -> bool {
        matches!(
            self.current_kind(),
            TokenKind::Int
                | TokenKind::Float
                | TokenKind::String_
                | TokenKind::Object
                | TokenKind::Mapping
                | TokenKind::Mixed
                | TokenKind::Void
        )
    }

    // ---- Statements ----

    fn parse_block(&mut self) -> Result<Block, ParseError> {
        self.expect(&TokenKind::LBrace)?;
        let mut stmts = Vec::new();

        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            stmts.push(self.parse_statement()?);
        }

        self.expect(&TokenKind::RBrace)?;
        Ok(stmts)
    }

    fn parse_statement(&mut self) -> Result<Stmt, ParseError> {
        match self.current_kind().clone() {
            TokenKind::If => self.parse_if_stmt(),
            TokenKind::While => self.parse_while_stmt(),
            TokenKind::Do => self.parse_do_while_stmt(),
            TokenKind::For => self.parse_for_stmt(),
            TokenKind::Switch => self.parse_switch_stmt(),
            TokenKind::Return => self.parse_return_stmt(),
            TokenKind::Break => self.parse_break_stmt(),
            TokenKind::Continue => self.parse_continue_stmt(),
            TokenKind::LBrace => self.parse_block_stmt(),
            TokenKind::Rlimits => self.parse_rlimits_stmt(),
            TokenKind::Catch => self.parse_catch_stmt(),
            _ => {
                // Try to parse as local variable declaration if we see a type keyword
                if self.is_local_var_decl() {
                    return self.parse_local_var_decl();
                }
                self.parse_expr_stmt()
            }
        }
    }

    /// Heuristic: is this a local variable declaration?
    /// A local var decl starts with optional modifiers then a type keyword, then an identifier,
    /// then either `=`, `;`, or `*` (array type then identifier).
    fn is_local_var_decl(&self) -> bool {
        let mut offset = 0;

        // Skip modifiers
        loop {
            match self.peek_kind(offset) {
                TokenKind::Private
                | TokenKind::Static
                | TokenKind::Nomask
                | TokenKind::Atomic
                | TokenKind::Varargs => {
                    offset += 1;
                }
                _ => break,
            }
        }

        // Must see a type keyword
        if !matches!(
            self.peek_kind(offset),
            TokenKind::Int
                | TokenKind::Float
                | TokenKind::String_
                | TokenKind::Object
                | TokenKind::Mapping
                | TokenKind::Mixed
                | TokenKind::Void
        ) {
            return false;
        }
        offset += 1;

        // Skip array stars
        while matches!(self.peek_kind(offset), TokenKind::Star) {
            offset += 1;
        }

        // Must be followed by an identifier
        matches!(self.peek_kind(offset), TokenKind::Identifier(_))
    }

    fn parse_local_var_decl(&mut self) -> Result<Stmt, ParseError> {
        let modifiers = self.parse_modifiers();
        let type_expr = self.parse_type_expr()?;
        let span = self.current_span();
        let name = self.expect_identifier()?;

        let initializer = if self.check(&TokenKind::Assign) {
            self.advance();
            Some(self.parse_assignment_expr()?)
        } else {
            None
        };

        self.expect(&TokenKind::Semicolon)?;

        Ok(Stmt::VarDecl(VarDecl {
            modifiers,
            type_expr,
            name,
            initializer,
            span,
        }))
    }

    fn parse_if_stmt(&mut self) -> Result<Stmt, ParseError> {
        let span = self.current_span();
        self.expect(&TokenKind::If)?;
        self.expect(&TokenKind::LParen)?;
        let condition = self.parse_expr()?;
        self.expect(&TokenKind::RParen)?;

        let then_branch = Box::new(self.parse_statement()?);
        let else_branch = if self.check(&TokenKind::Else) {
            self.advance();
            Some(Box::new(self.parse_statement()?))
        } else {
            None
        };

        Ok(Stmt::If(IfStmt {
            condition,
            then_branch,
            else_branch,
            span,
        }))
    }

    fn parse_while_stmt(&mut self) -> Result<Stmt, ParseError> {
        let span = self.current_span();
        self.expect(&TokenKind::While)?;
        self.expect(&TokenKind::LParen)?;
        let condition = self.parse_expr()?;
        self.expect(&TokenKind::RParen)?;
        let body = Box::new(self.parse_statement()?);

        Ok(Stmt::While(WhileStmt {
            condition,
            body,
            span,
        }))
    }

    fn parse_do_while_stmt(&mut self) -> Result<Stmt, ParseError> {
        let span = self.current_span();
        self.expect(&TokenKind::Do)?;
        let body = Box::new(self.parse_statement()?);
        self.expect(&TokenKind::While)?;
        self.expect(&TokenKind::LParen)?;
        let condition = self.parse_expr()?;
        self.expect(&TokenKind::RParen)?;
        self.expect(&TokenKind::Semicolon)?;

        Ok(Stmt::DoWhile(DoWhileStmt {
            body,
            condition,
            span,
        }))
    }

    fn parse_for_stmt(&mut self) -> Result<Stmt, ParseError> {
        let span = self.current_span();
        self.expect(&TokenKind::For)?;
        self.expect(&TokenKind::LParen)?;

        let init = if self.check(&TokenKind::Semicolon) {
            None
        } else {
            Some(Box::new(self.parse_expr()?))
        };
        self.expect(&TokenKind::Semicolon)?;

        let condition = if self.check(&TokenKind::Semicolon) {
            None
        } else {
            Some(Box::new(self.parse_expr()?))
        };
        self.expect(&TokenKind::Semicolon)?;

        let step = if self.check(&TokenKind::RParen) {
            None
        } else {
            Some(Box::new(self.parse_expr()?))
        };
        self.expect(&TokenKind::RParen)?;

        let body = Box::new(self.parse_statement()?);

        Ok(Stmt::For(ForStmt {
            init,
            condition,
            step,
            body,
            span,
        }))
    }

    fn parse_switch_stmt(&mut self) -> Result<Stmt, ParseError> {
        let span = self.current_span();
        self.expect(&TokenKind::Switch)?;
        self.expect(&TokenKind::LParen)?;
        let expr = self.parse_expr()?;
        self.expect(&TokenKind::RParen)?;
        self.expect(&TokenKind::LBrace)?;

        let mut cases = Vec::new();

        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            let case_span = self.current_span();

            let label = if self.check(&TokenKind::Case) {
                self.advance();
                let expr = self.parse_assignment_expr()?;

                if self.check(&TokenKind::DotDot) {
                    self.advance();
                    let end = self.parse_assignment_expr()?;
                    CaseLabel::Range(expr, end)
                } else {
                    CaseLabel::Expr(expr)
                }
            } else if self.check(&TokenKind::Default) {
                self.advance();
                CaseLabel::Default
            } else {
                let s = self.current_span();
                return Err(ParseError::UnexpectedToken {
                    found: self.current_kind().clone(),
                    expected: "'case' or 'default'".to_string(),
                    line: s.line,
                    col: s.col,
                });
            };

            self.expect(&TokenKind::Colon)?;

            let mut body = Vec::new();
            while !self.check(&TokenKind::Case)
                && !self.check(&TokenKind::Default)
                && !self.check(&TokenKind::RBrace)
                && !self.is_at_end()
            {
                body.push(self.parse_statement()?);
            }

            cases.push(SwitchCase {
                label,
                body,
                span: case_span,
            });
        }

        self.expect(&TokenKind::RBrace)?;

        Ok(Stmt::Switch(SwitchStmt { expr, cases, span }))
    }

    fn parse_return_stmt(&mut self) -> Result<Stmt, ParseError> {
        let span = self.current_span();
        self.expect(&TokenKind::Return)?;

        let value = if self.check(&TokenKind::Semicolon) {
            None
        } else {
            Some(self.parse_expr()?)
        };

        self.expect(&TokenKind::Semicolon)?;

        Ok(Stmt::Return(ReturnStmt { value, span }))
    }

    fn parse_break_stmt(&mut self) -> Result<Stmt, ParseError> {
        let span = self.current_span();
        self.expect(&TokenKind::Break)?;
        self.expect(&TokenKind::Semicolon)?;
        Ok(Stmt::Break(span))
    }

    fn parse_continue_stmt(&mut self) -> Result<Stmt, ParseError> {
        let span = self.current_span();
        self.expect(&TokenKind::Continue)?;
        self.expect(&TokenKind::Semicolon)?;
        Ok(Stmt::Continue(span))
    }

    fn parse_block_stmt(&mut self) -> Result<Stmt, ParseError> {
        let span = self.current_span();
        let stmts = self.parse_block()?;
        Ok(Stmt::Block(BlockStmt { stmts, span }))
    }

    fn parse_rlimits_stmt(&mut self) -> Result<Stmt, ParseError> {
        let span = self.current_span();
        self.expect(&TokenKind::Rlimits)?;
        self.expect(&TokenKind::LParen)?;
        let stack = self.parse_assignment_expr()?;
        self.expect(&TokenKind::Semicolon)?;
        let ticks = self.parse_assignment_expr()?;
        self.expect(&TokenKind::RParen)?;
        self.expect(&TokenKind::LBrace)?;

        let mut body = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            body.push(self.parse_statement()?);
        }
        self.expect(&TokenKind::RBrace)?;

        Ok(Stmt::Rlimits(RlimitsStmt {
            stack,
            ticks,
            body,
            span,
        }))
    }

    fn parse_catch_stmt(&mut self) -> Result<Stmt, ParseError> {
        let span = self.current_span();
        self.expect(&TokenKind::Catch)?;

        // Statement form: catch { ... } [: handler_expr]
        if self.check(&TokenKind::LBrace) {
            self.expect(&TokenKind::LBrace)?;
            let mut body = Vec::new();
            while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
                body.push(self.parse_statement()?);
            }
            self.expect(&TokenKind::RBrace)?;

            let handler = if self.check(&TokenKind::Colon) {
                self.advance();
                Some(self.parse_assignment_expr()?)
            } else {
                None
            };

            // Optional semicolon after catch statement
            if self.check(&TokenKind::Semicolon) {
                self.advance();
            }

            return Ok(Stmt::Catch(CatchStmt {
                body,
                handler,
                span,
            }));
        }

        // If not a block form, it might be catch(expr); used as a statement
        // Parse as expression statement with catch(expr)
        self.expect(&TokenKind::LParen)?;
        let inner = self.parse_expr()?;
        self.expect(&TokenKind::RParen)?;
        let expr_span = span;
        let catch_expr = Expr::CatchExpr(Box::new(inner), expr_span);
        self.expect(&TokenKind::Semicolon)?;

        Ok(Stmt::Expr(ExprStmt {
            expr: catch_expr,
            span,
        }))
    }

    fn parse_expr_stmt(&mut self) -> Result<Stmt, ParseError> {
        let span = self.current_span();
        let expr = self.parse_expr()?;
        self.expect(&TokenKind::Semicolon)?;
        Ok(Stmt::Expr(ExprStmt { expr, span }))
    }

    // ---- Expressions (Pratt parsing) ----

    /// Parse an expression (includes comma).
    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_expr_bp(BP_COMMA)
    }

    /// Parse an expression without comma (used in argument lists, initializers, etc.).
    fn parse_assignment_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_expr_bp(BP_ASSIGN)
    }

    /// Core Pratt expression parser.
    fn parse_expr_bp(&mut self, min_bp: Bp) -> Result<Expr, ParseError> {
        let mut left = self.parse_prefix()?;

        loop {
            if self.is_at_end() {
                break;
            }

            // Postfix operators (highest precedence among infixes)
            if let Some(expr) = self.try_parse_postfix(&left)? {
                left = expr;
                continue;
            }

            // Infix operators
            let Some((lbp, rbp, kind)) = self.infix_binding_power() else {
                break;
            };

            if lbp < min_bp {
                break;
            }

            left = self.parse_infix(left, rbp, kind)?;
        }

        Ok(left)
    }

    /// Classify the current token as an infix operator and return (left_bp, right_bp, kind).
    fn infix_binding_power(&self) -> Option<(Bp, Bp, InfixKind)> {
        let kind = self.current_kind();
        match kind {
            // Comma
            TokenKind::Comma => Some((BP_COMMA, Bp(BP_COMMA.0 + 1), InfixKind::Comma)),

            // Assignment (right-associative)
            TokenKind::Assign => Some((BP_ASSIGN, BP_ASSIGN, InfixKind::Assign(AssignOp::Assign))),
            TokenKind::PlusAssign => {
                Some((BP_ASSIGN, BP_ASSIGN, InfixKind::Assign(AssignOp::AddAssign)))
            }
            TokenKind::MinusAssign => {
                Some((BP_ASSIGN, BP_ASSIGN, InfixKind::Assign(AssignOp::SubAssign)))
            }
            TokenKind::StarAssign => {
                Some((BP_ASSIGN, BP_ASSIGN, InfixKind::Assign(AssignOp::MulAssign)))
            }
            TokenKind::SlashAssign => {
                Some((BP_ASSIGN, BP_ASSIGN, InfixKind::Assign(AssignOp::DivAssign)))
            }
            TokenKind::PercentAssign => {
                Some((BP_ASSIGN, BP_ASSIGN, InfixKind::Assign(AssignOp::ModAssign)))
            }
            TokenKind::AmpAssign => {
                Some((BP_ASSIGN, BP_ASSIGN, InfixKind::Assign(AssignOp::AndAssign)))
            }
            TokenKind::PipeAssign => {
                Some((BP_ASSIGN, BP_ASSIGN, InfixKind::Assign(AssignOp::OrAssign)))
            }
            TokenKind::CaretAssign => {
                Some((BP_ASSIGN, BP_ASSIGN, InfixKind::Assign(AssignOp::XorAssign)))
            }
            TokenKind::ShlAssign => {
                Some((BP_ASSIGN, BP_ASSIGN, InfixKind::Assign(AssignOp::ShlAssign)))
            }
            TokenKind::ShrAssign => {
                Some((BP_ASSIGN, BP_ASSIGN, InfixKind::Assign(AssignOp::ShrAssign)))
            }

            // Ternary (right-associative)
            TokenKind::Question => Some((BP_TERNARY, BP_TERNARY, InfixKind::Ternary)),

            // Logical OR
            TokenKind::OrOr => Some((BP_OR, Bp(BP_OR.0 + 1), InfixKind::Binary(BinaryOp::Or))),

            // Logical AND
            TokenKind::AndAnd => Some((BP_AND, Bp(BP_AND.0 + 1), InfixKind::Binary(BinaryOp::And))),

            // Bitwise OR
            TokenKind::Pipe => Some((
                BP_BIT_OR,
                Bp(BP_BIT_OR.0 + 1),
                InfixKind::Binary(BinaryOp::BitOr),
            )),

            // Bitwise XOR
            TokenKind::Caret => Some((
                BP_BIT_XOR,
                Bp(BP_BIT_XOR.0 + 1),
                InfixKind::Binary(BinaryOp::BitXor),
            )),

            // Bitwise AND
            TokenKind::Ampersand => Some((
                BP_BIT_AND,
                Bp(BP_BIT_AND.0 + 1),
                InfixKind::Binary(BinaryOp::BitAnd),
            )),

            // Equality
            TokenKind::EqEq => Some((
                BP_EQUALITY,
                Bp(BP_EQUALITY.0 + 1),
                InfixKind::Binary(BinaryOp::Eq),
            )),
            TokenKind::NotEq => Some((
                BP_EQUALITY,
                Bp(BP_EQUALITY.0 + 1),
                InfixKind::Binary(BinaryOp::NotEq),
            )),

            // Relational
            TokenKind::Less => Some((
                BP_RELATIONAL,
                Bp(BP_RELATIONAL.0 + 1),
                InfixKind::Binary(BinaryOp::Less),
            )),
            TokenKind::LessEq => Some((
                BP_RELATIONAL,
                Bp(BP_RELATIONAL.0 + 1),
                InfixKind::Binary(BinaryOp::LessEq),
            )),
            TokenKind::Greater => Some((
                BP_RELATIONAL,
                Bp(BP_RELATIONAL.0 + 1),
                InfixKind::Binary(BinaryOp::Greater),
            )),
            TokenKind::GreaterEq => Some((
                BP_RELATIONAL,
                Bp(BP_RELATIONAL.0 + 1),
                InfixKind::Binary(BinaryOp::GreaterEq),
            )),

            // Shift
            TokenKind::ShiftLeft => Some((
                BP_SHIFT,
                Bp(BP_SHIFT.0 + 1),
                InfixKind::Binary(BinaryOp::ShiftLeft),
            )),
            TokenKind::ShiftRight => Some((
                BP_SHIFT,
                Bp(BP_SHIFT.0 + 1),
                InfixKind::Binary(BinaryOp::ShiftRight),
            )),

            // Additive
            TokenKind::Plus => Some((
                BP_ADDITIVE,
                Bp(BP_ADDITIVE.0 + 1),
                InfixKind::Binary(BinaryOp::Add),
            )),
            TokenKind::Minus => Some((
                BP_ADDITIVE,
                Bp(BP_ADDITIVE.0 + 1),
                InfixKind::Binary(BinaryOp::Sub),
            )),

            // Multiplicative
            TokenKind::Star => Some((
                BP_MULTIPLICATIVE,
                Bp(BP_MULTIPLICATIVE.0 + 1),
                InfixKind::Binary(BinaryOp::Mul),
            )),
            TokenKind::Slash => Some((
                BP_MULTIPLICATIVE,
                Bp(BP_MULTIPLICATIVE.0 + 1),
                InfixKind::Binary(BinaryOp::Div),
            )),
            TokenKind::Percent => Some((
                BP_MULTIPLICATIVE,
                Bp(BP_MULTIPLICATIVE.0 + 1),
                InfixKind::Binary(BinaryOp::Mod),
            )),

            _ => None,
        }
    }

    /// Parse a prefix expression (NUD in Pratt terminology).
    fn parse_prefix(&mut self) -> Result<Expr, ParseError> {
        let kind = self.current_kind().clone();
        match kind {
            // Literals
            TokenKind::IntLiteral(v) => {
                let span = self.current_span();
                self.advance();
                Ok(Expr::IntLiteral(v, span))
            }
            TokenKind::FloatLiteral(v) => {
                let span = self.current_span();
                self.advance();
                Ok(Expr::FloatLiteral(v, span))
            }
            TokenKind::StringLiteral(ref s) => {
                let s = s.clone();
                let span = self.current_span();
                self.advance();
                Ok(Expr::StringLiteral(s, span))
            }
            TokenKind::CharLiteral(c) => {
                let span = self.current_span();
                self.advance();
                Ok(Expr::CharLiteral(c, span))
            }
            TokenKind::Nil => {
                let span = self.current_span();
                self.advance();
                Ok(Expr::NilLiteral(span))
            }

            // Identifier — could be simple ident, or label::func(...)
            TokenKind::Identifier(ref name) => {
                let name = name.clone();
                let span = self.current_span();
                self.advance();

                // label::func(args...)
                if self.check(&TokenKind::ColonColon) {
                    self.advance();
                    let func = self.expect_identifier()?;
                    let args = self.parse_call_args()?;
                    return Ok(Expr::ParentCall(ParentCallExpr {
                        label: Some(name),
                        function: func,
                        args,
                        span,
                    }));
                }

                Ok(Expr::Identifier(name, span))
            }

            // ::func(args...) — parent call without label
            TokenKind::ColonColon => {
                let span = self.current_span();
                self.advance();
                let func = self.expect_identifier()?;
                let args = self.parse_call_args()?;
                Ok(Expr::ParentCall(ParentCallExpr {
                    label: None,
                    function: func,
                    args,
                    span,
                }))
            }

            // Unary prefix: - ! ~ ++ --
            TokenKind::Minus => {
                let span = self.current_span();
                self.advance();
                let expr = self.parse_expr_bp(BP_UNARY)?;
                Ok(Expr::Unary(UnaryExpr {
                    op: UnaryOp::Neg,
                    expr: Box::new(expr),
                    span,
                }))
            }
            TokenKind::Bang => {
                let span = self.current_span();
                self.advance();
                let expr = self.parse_expr_bp(BP_UNARY)?;
                Ok(Expr::Unary(UnaryExpr {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                    span,
                }))
            }
            TokenKind::Tilde => {
                let span = self.current_span();
                self.advance();
                let expr = self.parse_expr_bp(BP_UNARY)?;
                Ok(Expr::Unary(UnaryExpr {
                    op: UnaryOp::BitNot,
                    expr: Box::new(expr),
                    span,
                }))
            }
            TokenKind::PlusPlus => {
                let span = self.current_span();
                self.advance();
                let expr = self.parse_expr_bp(BP_UNARY)?;
                Ok(Expr::Unary(UnaryExpr {
                    op: UnaryOp::PreIncrement,
                    expr: Box::new(expr),
                    span,
                }))
            }
            TokenKind::MinusMinus => {
                let span = self.current_span();
                self.advance();
                let expr = self.parse_expr_bp(BP_UNARY)?;
                Ok(Expr::Unary(UnaryExpr {
                    op: UnaryOp::PreDecrement,
                    expr: Box::new(expr),
                    span,
                }))
            }

            // sizeof(expr)
            TokenKind::Sizeof => {
                let span = self.current_span();
                self.advance();
                self.expect(&TokenKind::LParen)?;
                let inner = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(Expr::Sizeof(Box::new(inner), span))
            }

            // typeof(expr)
            TokenKind::Typeof => {
                let span = self.current_span();
                self.advance();
                self.expect(&TokenKind::LParen)?;
                let inner = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(Expr::Typeof(Box::new(inner), span))
            }

            // new(expr) or new_object(expr)
            TokenKind::New => {
                let span = self.current_span();
                self.advance();
                self.expect(&TokenKind::LParen)?;
                let inner = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(Expr::NewObject(Box::new(inner), span))
            }

            // catch(expr) — expression form
            TokenKind::Catch => {
                let span = self.current_span();
                self.advance();
                self.expect(&TokenKind::LParen)?;
                let inner = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(Expr::CatchExpr(Box::new(inner), span))
            }

            // Parenthesized expressions, type casts, and array literals
            TokenKind::LParen => self.parse_paren_expr(),

            // Mapping literal: ([ ... ])
            TokenKind::MappingOpen => self.parse_mapping_literal(),

            _ => {
                let span = self.current_span();
                Err(ParseError::UnexpectedToken {
                    found: kind,
                    expected: "expression".to_string(),
                    line: span.line,
                    col: span.col,
                })
            }
        }
    }

    /// Parse `(` ... `)` which could be:
    /// - `(expr)` — grouping
    /// - `(type)expr` — type cast
    /// - `({...})` — array literal
    fn parse_paren_expr(&mut self) -> Result<Expr, ParseError> {
        let span = self.current_span();
        self.expect(&TokenKind::LParen)?;

        // Array literal: ({...})
        if self.check(&TokenKind::LBrace) {
            self.advance(); // consume {
            let mut elements = Vec::new();

            if !self.check(&TokenKind::RBrace) {
                elements.push(self.parse_assignment_expr()?);
                while self.check(&TokenKind::Comma) {
                    self.advance();
                    // Allow trailing comma
                    if self.check(&TokenKind::RBrace) {
                        break;
                    }
                    elements.push(self.parse_assignment_expr()?);
                }
            }

            self.expect(&TokenKind::RBrace)?;
            self.expect(&TokenKind::RParen)?;
            return Ok(Expr::ArrayLiteral(elements, span));
        }

        // Type cast: (type)expr or (type *)expr
        if self.is_type_keyword() {
            // Try to parse as cast: save position, try type + RParen
            let saved_pos = self.pos;
            if let Ok(type_expr) = self.parse_type_expr() {
                if self.check(&TokenKind::RParen) {
                    self.advance(); // consume )
                    let expr = self.parse_expr_bp(BP_UNARY)?;
                    return Ok(Expr::Cast(CastExpr {
                        type_expr,
                        expr: Box::new(expr),
                        span,
                    }));
                }
            }
            // Not a cast, backtrack and parse as grouping
            self.pos = saved_pos;
        }

        // Grouping: (expr)
        let expr = self.parse_expr()?;
        self.expect(&TokenKind::RParen)?;
        Ok(expr)
    }

    /// Parse mapping literal: ([ key: value, ... ])
    fn parse_mapping_literal(&mut self) -> Result<Expr, ParseError> {
        let span = self.current_span();
        self.expect(&TokenKind::MappingOpen)?;

        let mut pairs = Vec::new();
        if !self.check(&TokenKind::MappingClose) {
            let key = self.parse_assignment_expr()?;
            self.expect(&TokenKind::Colon)?;
            let value = self.parse_assignment_expr()?;
            pairs.push((key, value));

            while self.check(&TokenKind::Comma) {
                self.advance();
                if self.check(&TokenKind::MappingClose) {
                    break;
                }
                let key = self.parse_assignment_expr()?;
                self.expect(&TokenKind::Colon)?;
                let value = self.parse_assignment_expr()?;
                pairs.push((key, value));
            }
        }

        self.expect(&TokenKind::MappingClose)?;
        Ok(Expr::MappingLiteral(pairs, span))
    }

    /// Try to parse a postfix operation on `left`. Returns `Some(new_expr)` if
    /// a postfix was consumed, `None` if the current token is not postfix.
    fn try_parse_postfix(&mut self, left: &Expr) -> Result<Option<Expr>, ParseError> {
        match self.current_kind() {
            // Function call: expr(args...)
            TokenKind::LParen => {
                let span = self.current_span();
                let args = self.parse_call_args()?;
                Ok(Some(Expr::Call(CallExpr {
                    function: Box::new(left.clone()),
                    args,
                    span,
                })))
            }

            // Index or range: expr[index] or expr[start..end]
            TokenKind::LBracket => {
                let span = self.current_span();
                self.advance(); // consume [

                // Range with no start: expr[..end]
                if self.check(&TokenKind::DotDot) {
                    self.advance();
                    let end = if self.check(&TokenKind::RBracket) {
                        None
                    } else {
                        Some(Box::new(self.parse_expr()?))
                    };
                    self.expect(&TokenKind::RBracket)?;
                    return Ok(Some(Expr::Range(RangeExpr {
                        object: Box::new(left.clone()),
                        start: None,
                        end,
                        span,
                    })));
                }

                let index = self.parse_expr()?;

                // Range: expr[start..end]
                if self.check(&TokenKind::DotDot) {
                    self.advance();
                    let end = if self.check(&TokenKind::RBracket) {
                        None
                    } else {
                        Some(Box::new(self.parse_expr()?))
                    };
                    self.expect(&TokenKind::RBracket)?;
                    return Ok(Some(Expr::Range(RangeExpr {
                        object: Box::new(left.clone()),
                        start: Some(Box::new(index)),
                        end,
                        span,
                    })));
                }

                self.expect(&TokenKind::RBracket)?;
                Ok(Some(Expr::Index(IndexExpr {
                    object: Box::new(left.clone()),
                    index: Box::new(index),
                    span,
                })))
            }

            // Call other: expr->method(args...)
            TokenKind::Arrow => {
                let span = self.current_span();
                self.advance(); // consume ->
                let method = self.expect_identifier()?;
                let args = self.parse_call_args()?;
                Ok(Some(Expr::CallOther(CallOtherExpr {
                    object: Box::new(left.clone()),
                    method,
                    args,
                    span,
                })))
            }

            // Post-increment: expr++
            TokenKind::PlusPlus => {
                let span = self.current_span();
                self.advance();
                Ok(Some(Expr::PostIncrement(Box::new(left.clone()), span)))
            }

            // Post-decrement: expr--
            TokenKind::MinusMinus => {
                let span = self.current_span();
                self.advance();
                Ok(Some(Expr::PostDecrement(Box::new(left.clone()), span)))
            }

            _ => Ok(None),
        }
    }

    /// Parse the infix portion (LED in Pratt terminology).
    fn parse_infix(&mut self, left: Expr, rbp: Bp, kind: InfixKind) -> Result<Expr, ParseError> {
        match kind {
            InfixKind::Binary(op) => {
                let span = self.current_span();
                self.advance(); // consume operator token
                let right = self.parse_expr_bp(rbp)?;
                Ok(Expr::Binary(BinaryExpr {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                    span,
                }))
            }
            InfixKind::Assign(op) => {
                let span = self.current_span();
                self.advance();
                let right = self.parse_expr_bp(rbp)?;
                Ok(Expr::Assign(AssignExpr {
                    target: Box::new(left),
                    op,
                    value: Box::new(right),
                    span,
                }))
            }
            InfixKind::Ternary => {
                let span = self.current_span();
                self.advance(); // consume ?
                let then_expr = self.parse_expr_bp(BP_ASSIGN)?;
                self.expect(&TokenKind::Colon)?;
                let else_expr = self.parse_expr_bp(rbp)?;
                Ok(Expr::Ternary(TernaryExpr {
                    condition: Box::new(left),
                    then_expr: Box::new(then_expr),
                    else_expr: Box::new(else_expr),
                    span,
                }))
            }
            InfixKind::Comma => {
                let span = self.current_span();
                self.advance(); // consume ,
                let right = self.parse_expr_bp(rbp)?;

                // Flatten comma expressions
                let mut exprs = match left {
                    Expr::Comma(mut v, _) => {
                        v.push(right);
                        v
                    }
                    _ => vec![left, right],
                };

                // Continue flattening
                while self.check(&TokenKind::Comma) {
                    self.advance();
                    exprs.push(self.parse_expr_bp(rbp)?);
                }

                Ok(Expr::Comma(exprs, span))
            }
        }
    }

    /// Parse call arguments: `(arg1, arg2, ...)`
    fn parse_call_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        self.expect(&TokenKind::LParen)?;
        let mut args = Vec::new();

        if !self.check(&TokenKind::RParen) {
            args.push(self.parse_assignment_expr()?);
            while self.check(&TokenKind::Comma) {
                self.advance();
                args.push(self.parse_assignment_expr()?);
            }
        }

        self.expect(&TokenKind::RParen)?;
        Ok(args)
    }

    // ---- Token helpers ----

    fn is_at_end(&self) -> bool {
        self.pos >= self.tokens.len() || self.tokens[self.pos].kind == TokenKind::Eof
    }

    fn current_kind(&self) -> &TokenKind {
        if self.pos < self.tokens.len() {
            &self.tokens[self.pos].kind
        } else {
            &TokenKind::Eof
        }
    }

    fn current_span(&self) -> Span {
        if self.pos < self.tokens.len() {
            self.tokens[self.pos].span
        } else {
            Span::dummy()
        }
    }

    /// Check if the current token matches `kind` (without consuming).
    fn check(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(self.current_kind()) == std::mem::discriminant(kind)
    }

    fn check_identifier(&self) -> bool {
        matches!(self.current_kind(), TokenKind::Identifier(_))
    }

    fn check_ahead(&self, offset: usize, kind: &TokenKind) -> bool {
        let idx = self.pos + offset;
        if idx < self.tokens.len() {
            std::mem::discriminant(&self.tokens[idx].kind) == std::mem::discriminant(kind)
        } else {
            false
        }
    }

    fn check_ahead_string_literal(&self, offset: usize) -> bool {
        let idx = self.pos + offset;
        if idx < self.tokens.len() {
            matches!(self.tokens[idx].kind, TokenKind::StringLiteral(_))
        } else {
            false
        }
    }

    /// Get the token kind at pos + offset.
    fn peek_kind(&self, offset: usize) -> &TokenKind {
        let idx = self.pos + offset;
        if idx < self.tokens.len() {
            &self.tokens[idx].kind
        } else {
            &TokenKind::Eof
        }
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos];
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, kind: &TokenKind) -> Result<&Token, ParseError> {
        if self.is_at_end() && *kind != TokenKind::Eof {
            return Err(ParseError::UnexpectedEof {
                expected: format!("{:?}", kind),
            });
        }
        if self.check(kind) {
            Ok(self.advance())
        } else {
            let span = self.current_span();
            Err(ParseError::UnexpectedToken {
                found: self.current_kind().clone(),
                expected: format!("{:?}", kind),
                line: span.line,
                col: span.col,
            })
        }
    }

    fn expect_identifier(&mut self) -> Result<String, ParseError> {
        if let TokenKind::Identifier(ref name) = self.current_kind().clone() {
            let name = name.clone();
            self.advance();
            Ok(name)
        } else {
            let span = self.current_span();
            Err(ParseError::UnexpectedToken {
                found: self.current_kind().clone(),
                expected: "identifier".to_string(),
                line: span.line,
                col: span.col,
            })
        }
    }

    fn expect_string_literal(&mut self) -> Result<String, ParseError> {
        if let TokenKind::StringLiteral(ref s) = self.current_kind().clone() {
            let s = s.clone();
            self.advance();
            Ok(s)
        } else {
            let span = self.current_span();
            Err(ParseError::UnexpectedToken {
                found: self.current_kind().clone(),
                expected: "string literal".to_string(),
                line: span.line,
                col: span.col,
            })
        }
    }
}

/// Classification of infix operators for the Pratt parser.
#[derive(Debug, Clone, Copy)]
enum InfixKind {
    Binary(BinaryOp),
    Assign(AssignOp),
    Ternary,
    Comma,
}
