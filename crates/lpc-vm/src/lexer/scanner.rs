use super::token::{keyword_lookup, Span, Token, TokenKind};

/// Lexer error types.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LexError {
    #[error("unexpected character '{ch}' at line {line}, col {col}")]
    UnexpectedChar { ch: char, line: u32, col: u32 },

    #[error("unterminated string literal at line {line}, col {col}")]
    UnterminatedString { line: u32, col: u32 },

    #[error("unterminated char literal at line {line}, col {col}")]
    UnterminatedChar { line: u32, col: u32 },

    #[error("unterminated block comment at line {line}, col {col}")]
    UnterminatedComment { line: u32, col: u32 },

    #[error("invalid escape sequence '\\{ch}' at line {line}, col {col}")]
    InvalidEscape { ch: char, line: u32, col: u32 },

    #[error("invalid number literal at line {line}, col {col}")]
    InvalidNumber { line: u32, col: u32 },
}

/// Hand-written scanner for LPC source code.
pub struct Scanner {
    source: Vec<char>,
    pos: usize,
    line: u32,
    col: u32,
}

impl Scanner {
    pub fn new(source: &str) -> Self {
        Self {
            source: source.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    /// Produce the next token, or an error.
    pub fn next_token(&mut self) -> Result<Token, LexError> {
        self.skip_whitespace_and_comments()?;

        if self.is_at_end() {
            return Ok(Token::new(TokenKind::Eof, self.current_span(0), ""));
        }

        let start = self.pos;
        let start_line = self.line;
        let start_col = self.col;
        let ch = self.peek();

        // Number literals
        if ch.is_ascii_digit() || (ch == '.' && self.peek_next().is_some_and(|c| c.is_ascii_digit())) {
            return self.scan_number(start, start_line, start_col);
        }

        // String literal
        if ch == '"' {
            return self.scan_string(start, start_line, start_col);
        }

        // Char literal
        if ch == '\'' {
            return self.scan_char(start, start_line, start_col);
        }

        // Identifier or keyword
        if ch.is_ascii_alphabetic() || ch == '_' {
            return self.scan_identifier(start, start_line, start_col);
        }

        // Operators and delimiters
        self.scan_operator(start, start_line, start_col)
    }

    /// Tokenize the entire source into a Vec of tokens (ending with Eof).
    pub fn scan_all(&mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token()?;
            let is_eof = tok.kind == TokenKind::Eof;
            tokens.push(tok);
            if is_eof {
                break;
            }
        }
        Ok(tokens)
    }

    // ---- Helpers ----

    fn is_at_end(&self) -> bool {
        self.pos >= self.source.len()
    }

    fn peek(&self) -> char {
        self.source[self.pos]
    }

    fn peek_next(&self) -> Option<char> {
        self.source.get(self.pos + 1).copied()
    }

    fn advance(&mut self) -> char {
        let ch = self.source[self.pos];
        self.pos += 1;
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        ch
    }

    fn current_span(&self, len: usize) -> Span {
        Span::new(self.pos, self.pos + len, self.line, self.col)
    }

    fn make_span(&self, start: usize, start_line: u32, start_col: u32) -> Span {
        Span::new(start, self.pos, start_line, start_col)
    }

    fn text_from(&self, start: usize) -> String {
        self.source[start..self.pos].iter().collect()
    }

    fn skip_whitespace_and_comments(&mut self) -> Result<(), LexError> {
        loop {
            // Skip whitespace
            while !self.is_at_end() && self.peek().is_ascii_whitespace() {
                self.advance();
            }

            if self.is_at_end() {
                return Ok(());
            }

            // Line comments
            if self.peek() == '/' && self.peek_next() == Some('/') {
                while !self.is_at_end() && self.peek() != '\n' {
                    self.advance();
                }
                continue;
            }

            // Block comments
            if self.peek() == '/' && self.peek_next() == Some('*') {
                let comment_line = self.line;
                let comment_col = self.col;
                self.advance(); // /
                self.advance(); // *
                loop {
                    if self.is_at_end() {
                        return Err(LexError::UnterminatedComment {
                            line: comment_line,
                            col: comment_col,
                        });
                    }
                    if self.peek() == '*' && self.peek_next() == Some('/') {
                        self.advance(); // *
                        self.advance(); // /
                        break;
                    }
                    self.advance();
                }
                continue;
            }

            break;
        }
        Ok(())
    }

    fn scan_number(&mut self, start: usize, start_line: u32, start_col: u32) -> Result<Token, LexError> {
        let first = self.peek();

        // Leading dot: must be a float like .5
        if first == '.' {
            return self.scan_float_from_dot(start, start_line, start_col);
        }

        // Hex: 0x...
        if first == '0' && self.peek_next().is_some_and(|c| c == 'x' || c == 'X') {
            self.advance(); // 0
            self.advance(); // x
            if self.is_at_end() || !self.peek().is_ascii_hexdigit() {
                return Err(LexError::InvalidNumber {
                    line: start_line,
                    col: start_col,
                });
            }
            while !self.is_at_end() && self.peek().is_ascii_hexdigit() {
                self.advance();
            }
            let text = self.text_from(start);
            let value = i64::from_str_radix(&text[2..], 16).map_err(|_| LexError::InvalidNumber {
                line: start_line,
                col: start_col,
            })?;
            return Ok(Token::new(
                TokenKind::IntLiteral(value),
                self.make_span(start, start_line, start_col),
                text,
            ));
        }

        // Octal: starts with 0 and followed by digits
        if first == '0' && self.peek_next().is_some_and(|c| c.is_ascii_digit()) {
            self.advance(); // 0
            while !self.is_at_end() && self.peek().is_ascii_digit() {
                self.advance();
            }
            // Check if it becomes a float
            if !self.is_at_end() && (self.peek() == '.' || self.peek() == 'e' || self.peek() == 'E') {
                return self.continue_float(start, start_line, start_col);
            }
            let text = self.text_from(start);
            let value = i64::from_str_radix(&text[1..], 8).map_err(|_| LexError::InvalidNumber {
                line: start_line,
                col: start_col,
            })?;
            return Ok(Token::new(
                TokenKind::IntLiteral(value),
                self.make_span(start, start_line, start_col),
                text,
            ));
        }

        // Decimal integer or float
        while !self.is_at_end() && self.peek().is_ascii_digit() {
            self.advance();
        }

        // Check for float
        if !self.is_at_end() && (self.peek() == '.' || self.peek() == 'e' || self.peek() == 'E') {
            // But not .. (range operator)
            if self.peek() == '.' && self.peek_next() == Some('.') {
                // It's an integer followed by ..
                let text = self.text_from(start);
                let value: i64 = text.parse().map_err(|_| LexError::InvalidNumber {
                    line: start_line,
                    col: start_col,
                })?;
                return Ok(Token::new(
                    TokenKind::IntLiteral(value),
                    self.make_span(start, start_line, start_col),
                    text,
                ));
            }
            return self.continue_float(start, start_line, start_col);
        }

        let text = self.text_from(start);
        let value: i64 = text.parse().map_err(|_| LexError::InvalidNumber {
            line: start_line,
            col: start_col,
        })?;
        Ok(Token::new(
            TokenKind::IntLiteral(value),
            self.make_span(start, start_line, start_col),
            text,
        ))
    }

    fn scan_float_from_dot(&mut self, start: usize, start_line: u32, start_col: u32) -> Result<Token, LexError> {
        self.advance(); // .
        while !self.is_at_end() && self.peek().is_ascii_digit() {
            self.advance();
        }
        // Exponent
        if !self.is_at_end() && (self.peek() == 'e' || self.peek() == 'E') {
            self.advance();
            if !self.is_at_end() && (self.peek() == '+' || self.peek() == '-') {
                self.advance();
            }
            if self.is_at_end() || !self.peek().is_ascii_digit() {
                return Err(LexError::InvalidNumber {
                    line: start_line,
                    col: start_col,
                });
            }
            while !self.is_at_end() && self.peek().is_ascii_digit() {
                self.advance();
            }
        }
        let text = self.text_from(start);
        let value: f64 = text.parse().map_err(|_| LexError::InvalidNumber {
            line: start_line,
            col: start_col,
        })?;
        Ok(Token::new(
            TokenKind::FloatLiteral(value),
            self.make_span(start, start_line, start_col),
            text,
        ))
    }

    /// Continue scanning a float after we've already consumed some integer digits.
    fn continue_float(&mut self, start: usize, start_line: u32, start_col: u32) -> Result<Token, LexError> {
        if !self.is_at_end() && self.peek() == '.' {
            self.advance(); // .
            while !self.is_at_end() && self.peek().is_ascii_digit() {
                self.advance();
            }
        }
        // Exponent
        if !self.is_at_end() && (self.peek() == 'e' || self.peek() == 'E') {
            self.advance();
            if !self.is_at_end() && (self.peek() == '+' || self.peek() == '-') {
                self.advance();
            }
            if self.is_at_end() || !self.peek().is_ascii_digit() {
                return Err(LexError::InvalidNumber {
                    line: start_line,
                    col: start_col,
                });
            }
            while !self.is_at_end() && self.peek().is_ascii_digit() {
                self.advance();
            }
        }
        let text = self.text_from(start);
        let value: f64 = text.parse().map_err(|_| LexError::InvalidNumber {
            line: start_line,
            col: start_col,
        })?;
        Ok(Token::new(
            TokenKind::FloatLiteral(value),
            self.make_span(start, start_line, start_col),
            text,
        ))
    }

    fn scan_escape(&mut self, start_line: u32, start_col: u32) -> Result<char, LexError> {
        if self.is_at_end() {
            return Err(LexError::InvalidEscape {
                ch: ' ',
                line: start_line,
                col: start_col,
            });
        }
        let ch = self.advance();
        match ch {
            'n' => Ok('\n'),
            't' => Ok('\t'),
            'r' => Ok('\r'),
            '\\' => Ok('\\'),
            '"' => Ok('"'),
            '\'' => Ok('\''),
            '0' => Ok('\0'),
            'x' => {
                // Hex escape: \xNN
                let mut hex = String::new();
                for _ in 0..2 {
                    if self.is_at_end() || !self.peek().is_ascii_hexdigit() {
                        return Err(LexError::InvalidEscape {
                            ch: 'x',
                            line: start_line,
                            col: start_col,
                        });
                    }
                    hex.push(self.advance());
                }
                let code = u8::from_str_radix(&hex, 16).map_err(|_| LexError::InvalidEscape {
                    ch: 'x',
                    line: start_line,
                    col: start_col,
                })?;
                Ok(code as char)
            }
            other => Err(LexError::InvalidEscape {
                ch: other,
                line: start_line,
                col: start_col,
            }),
        }
    }

    fn scan_string(&mut self, start: usize, start_line: u32, start_col: u32) -> Result<Token, LexError> {
        self.advance(); // opening "
        let mut value = String::new();
        loop {
            if self.is_at_end() {
                return Err(LexError::UnterminatedString {
                    line: start_line,
                    col: start_col,
                });
            }
            let ch = self.peek();
            if ch == '"' {
                self.advance(); // closing "
                let text = self.text_from(start);
                return Ok(Token::new(
                    TokenKind::StringLiteral(value),
                    self.make_span(start, start_line, start_col),
                    text,
                ));
            }
            if ch == '\n' {
                return Err(LexError::UnterminatedString {
                    line: start_line,
                    col: start_col,
                });
            }
            if ch == '\\' {
                self.advance(); // backslash
                let escaped = self.scan_escape(start_line, start_col)?;
                value.push(escaped);
            } else {
                value.push(self.advance());
            }
        }
    }

    fn scan_char(&mut self, start: usize, start_line: u32, start_col: u32) -> Result<Token, LexError> {
        self.advance(); // opening '
        if self.is_at_end() {
            return Err(LexError::UnterminatedChar {
                line: start_line,
                col: start_col,
            });
        }
        let value = if self.peek() == '\\' {
            self.advance(); // backslash
            self.scan_escape(start_line, start_col)?
        } else {
            self.advance()
        };
        if self.is_at_end() || self.peek() != '\'' {
            return Err(LexError::UnterminatedChar {
                line: start_line,
                col: start_col,
            });
        }
        self.advance(); // closing '
        let text = self.text_from(start);
        Ok(Token::new(
            TokenKind::CharLiteral(value),
            self.make_span(start, start_line, start_col),
            text,
        ))
    }

    fn scan_identifier(&mut self, start: usize, start_line: u32, start_col: u32) -> Result<Token, LexError> {
        while !self.is_at_end() && (self.peek().is_ascii_alphanumeric() || self.peek() == '_') {
            self.advance();
        }
        let text = self.text_from(start);
        let kind = if let Some(kw) = keyword_lookup(&text) {
            kw
        } else {
            TokenKind::Identifier(text.clone())
        };
        Ok(Token::new(
            kind,
            self.make_span(start, start_line, start_col),
            text,
        ))
    }

    fn emit(&self, kind: TokenKind, start: usize, start_line: u32, start_col: u32) -> Result<Token, LexError> {
        let text = self.text_from(start);
        Ok(Token::new(kind, self.make_span(start, start_line, start_col), text))
    }

    fn scan_operator(&mut self, start: usize, start_line: u32, start_col: u32) -> Result<Token, LexError> {
        let ch = self.advance();

        match ch {
            '(' => {
                if !self.is_at_end() && self.peek() == '[' {
                    self.advance();
                    return self.emit(TokenKind::MappingOpen, start, start_line, start_col);
                }
                self.emit(TokenKind::LParen, start, start_line, start_col)
            }
            ')' => self.emit(TokenKind::RParen, start, start_line, start_col),
            '{' => self.emit(TokenKind::LBrace, start, start_line, start_col),
            '}' => self.emit(TokenKind::RBrace, start, start_line, start_col),
            '[' => self.emit(TokenKind::LBracket, start, start_line, start_col),
            ']' => {
                if !self.is_at_end() && self.peek() == ')' {
                    self.advance();
                    return self.emit(TokenKind::MappingClose, start, start_line, start_col);
                }
                self.emit(TokenKind::RBracket, start, start_line, start_col)
            }
            ';' => self.emit(TokenKind::Semicolon, start, start_line, start_col),
            ',' => self.emit(TokenKind::Comma, start, start_line, start_col),
            '~' => self.emit(TokenKind::Tilde, start, start_line, start_col),
            '?' => self.emit(TokenKind::Question, start, start_line, start_col),

            '+' => {
                if !self.is_at_end() {
                    match self.peek() {
                        '+' => { self.advance(); return self.emit(TokenKind::PlusPlus, start, start_line, start_col); }
                        '=' => { self.advance(); return self.emit(TokenKind::PlusAssign, start, start_line, start_col); }
                        _ => {}
                    }
                }
                self.emit(TokenKind::Plus, start, start_line, start_col)
            }

            '-' => {
                if !self.is_at_end() {
                    match self.peek() {
                        '-' => { self.advance(); return self.emit(TokenKind::MinusMinus, start, start_line, start_col); }
                        '>' => { self.advance(); return self.emit(TokenKind::Arrow, start, start_line, start_col); }
                        '=' => { self.advance(); return self.emit(TokenKind::MinusAssign, start, start_line, start_col); }
                        _ => {}
                    }
                }
                self.emit(TokenKind::Minus, start, start_line, start_col)
            }

            '*' => {
                if !self.is_at_end() && self.peek() == '=' {
                    self.advance();
                    return self.emit(TokenKind::StarAssign, start, start_line, start_col);
                }
                self.emit(TokenKind::Star, start, start_line, start_col)
            }

            '/' => {
                if !self.is_at_end() && self.peek() == '=' {
                    self.advance();
                    return self.emit(TokenKind::SlashAssign, start, start_line, start_col);
                }
                self.emit(TokenKind::Slash, start, start_line, start_col)
            }

            '%' => {
                if !self.is_at_end() && self.peek() == '=' {
                    self.advance();
                    return self.emit(TokenKind::PercentAssign, start, start_line, start_col);
                }
                self.emit(TokenKind::Percent, start, start_line, start_col)
            }

            '&' => {
                if !self.is_at_end() {
                    match self.peek() {
                        '&' => { self.advance(); return self.emit(TokenKind::AndAnd, start, start_line, start_col); }
                        '=' => { self.advance(); return self.emit(TokenKind::AmpAssign, start, start_line, start_col); }
                        _ => {}
                    }
                }
                self.emit(TokenKind::Ampersand, start, start_line, start_col)
            }

            '|' => {
                if !self.is_at_end() {
                    match self.peek() {
                        '|' => { self.advance(); return self.emit(TokenKind::OrOr, start, start_line, start_col); }
                        '=' => { self.advance(); return self.emit(TokenKind::PipeAssign, start, start_line, start_col); }
                        _ => {}
                    }
                }
                self.emit(TokenKind::Pipe, start, start_line, start_col)
            }

            '^' => {
                if !self.is_at_end() && self.peek() == '=' {
                    self.advance();
                    return self.emit(TokenKind::CaretAssign, start, start_line, start_col);
                }
                self.emit(TokenKind::Caret, start, start_line, start_col)
            }

            '!' => {
                if !self.is_at_end() && self.peek() == '=' {
                    self.advance();
                    return self.emit(TokenKind::NotEq, start, start_line, start_col);
                }
                self.emit(TokenKind::Bang, start, start_line, start_col)
            }

            '=' => {
                if !self.is_at_end() && self.peek() == '=' {
                    self.advance();
                    return self.emit(TokenKind::EqEq, start, start_line, start_col);
                }
                self.emit(TokenKind::Assign, start, start_line, start_col)
            }

            '<' => {
                if !self.is_at_end() {
                    match self.peek() {
                        '<' => {
                            self.advance();
                            if !self.is_at_end() && self.peek() == '=' {
                                self.advance();
                                return self.emit(TokenKind::ShlAssign, start, start_line, start_col);
                            }
                            return self.emit(TokenKind::ShiftLeft, start, start_line, start_col);
                        }
                        '=' => { self.advance(); return self.emit(TokenKind::LessEq, start, start_line, start_col); }
                        _ => {}
                    }
                }
                self.emit(TokenKind::Less, start, start_line, start_col)
            }

            '>' => {
                if !self.is_at_end() {
                    match self.peek() {
                        '>' => {
                            self.advance();
                            if !self.is_at_end() && self.peek() == '=' {
                                self.advance();
                                return self.emit(TokenKind::ShrAssign, start, start_line, start_col);
                            }
                            return self.emit(TokenKind::ShiftRight, start, start_line, start_col);
                        }
                        '=' => { self.advance(); return self.emit(TokenKind::GreaterEq, start, start_line, start_col); }
                        _ => {}
                    }
                }
                self.emit(TokenKind::Greater, start, start_line, start_col)
            }

            ':' => {
                if !self.is_at_end() && self.peek() == ':' {
                    self.advance();
                    return self.emit(TokenKind::ColonColon, start, start_line, start_col);
                }
                self.emit(TokenKind::Colon, start, start_line, start_col)
            }

            '.' => {
                if !self.is_at_end() && self.peek() == '.' {
                    self.advance();
                    if !self.is_at_end() && self.peek() == '.' {
                        self.advance();
                        return self.emit(TokenKind::Ellipsis, start, start_line, start_col);
                    }
                    return self.emit(TokenKind::DotDot, start, start_line, start_col);
                }
                self.emit(TokenKind::Dot, start, start_line, start_col)
            }

            _ => Err(LexError::UnexpectedChar {
                ch,
                line: start_line,
                col: start_col,
            }),
        }
    }
}
