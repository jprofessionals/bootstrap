use std::collections::HashMap;

/// Preprocessor error types.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PreprocessError {
    #[error("unknown directive '#{directive}' at {filename}:{line}")]
    UnknownDirective {
        directive: String,
        filename: String,
        line: u32,
    },

    #[error("unterminated conditional at {filename}:{line}")]
    UnterminatedConditional { filename: String, line: u32 },

    #[error("#else without #if at {filename}:{line}")]
    ElseWithoutIf { filename: String, line: u32 },

    #[error("#elif without #if at {filename}:{line}")]
    ElifWithoutIf { filename: String, line: u32 },

    #[error("#endif without #if at {filename}:{line}")]
    EndifWithoutIf { filename: String, line: u32 },

    #[error("#error: {message} at {filename}:{line}")]
    UserError {
        message: String,
        filename: String,
        line: u32,
    },

    #[error("could not include '{path}' at {filename}:{line}")]
    IncludeNotFound {
        path: String,
        filename: String,
        line: u32,
    },

    #[error("invalid macro definition at {filename}:{line}")]
    InvalidMacro { filename: String, line: u32 },

    #[error("invalid expression in #if at {filename}:{line}: {detail}")]
    InvalidExpression {
        detail: String,
        filename: String,
        line: u32,
    },

    #[error("macro argument count mismatch for '{name}' at {filename}:{line}")]
    MacroArgMismatch {
        name: String,
        filename: String,
        line: u32,
    },
}

/// A macro definition: either a simple text substitution or parameterized.
#[derive(Debug, Clone)]
enum MacroDef {
    Simple(String),
    Parameterized { params: Vec<String>, body: String },
}

/// State for nested #if / #ifdef / #ifndef / #else / #elif blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CondState {
    /// We are inside a branch that is active (emitting output).
    Active,
    /// We are inside a branch that was skipped.
    Inactive,
    /// We already found a true branch in this group, so skip the rest.
    Done,
}

/// C-style preprocessor for LPC source files.
pub struct Preprocessor {
    defines: HashMap<String, MacroDef>,
    include_resolver: Option<Box<dyn Fn(&str) -> Option<String>>>,
}

impl Preprocessor {
    pub fn new() -> Self {
        Self {
            defines: HashMap::new(),
            include_resolver: None,
        }
    }

    /// Set the callback used to resolve `#include` paths to source text.
    pub fn set_include_resolver(&mut self, resolver: Box<dyn Fn(&str) -> Option<String>>) {
        self.include_resolver = Some(resolver);
    }

    /// Pre-define a macro.
    pub fn define(&mut self, name: &str, value: &str) {
        self.defines
            .insert(name.to_string(), MacroDef::Simple(value.to_string()));
    }

    /// Process the source, expanding all preprocessor directives.
    pub fn process(&mut self, source: &str, filename: &str) -> Result<String, PreprocessError> {
        let source = join_line_continuations(source);
        let lines: Vec<&str> = source.lines().collect();
        let mut output = String::new();
        let mut cond_stack: Vec<CondState> = Vec::new();
        let mut line_num: u32 = 0;

        let mut i = 0;
        while i < lines.len() {
            line_num += 1;
            let raw_line = lines[i];
            let trimmed = raw_line.trim();
            i += 1;

            if let Some(directive_text) = trimmed.strip_prefix('#') {
                let directive_text = directive_text.trim();

                // Parse directive name
                let (directive, rest) = split_directive(directive_text);

                match directive {
                    "define" => {
                        if !is_active(&cond_stack) {
                            continue;
                        }
                        self.handle_define(rest, filename, line_num)?;
                    }
                    "undef" => {
                        if !is_active(&cond_stack) {
                            continue;
                        }
                        let name = rest.trim();
                        self.defines.remove(name);
                    }
                    "ifdef" => {
                        let name = rest.trim();
                        if !is_active(&cond_stack) {
                            cond_stack.push(CondState::Inactive);
                        } else if self.defines.contains_key(name) {
                            cond_stack.push(CondState::Active);
                        } else {
                            cond_stack.push(CondState::Inactive);
                        }
                    }
                    "ifndef" => {
                        let name = rest.trim();
                        if !is_active(&cond_stack) {
                            cond_stack.push(CondState::Inactive);
                        } else if !self.defines.contains_key(name) {
                            cond_stack.push(CondState::Active);
                        } else {
                            cond_stack.push(CondState::Inactive);
                        }
                    }
                    "if" => {
                        if !is_active(&cond_stack) {
                            cond_stack.push(CondState::Inactive);
                        } else {
                            let val = self.eval_condition(rest, filename, line_num)?;
                            if val {
                                cond_stack.push(CondState::Active);
                            } else {
                                cond_stack.push(CondState::Inactive);
                            }
                        }
                    }
                    "elif" => {
                        if cond_stack.is_empty() {
                            return Err(PreprocessError::ElifWithoutIf {
                                filename: filename.to_string(),
                                line: line_num,
                            });
                        }
                        let len = cond_stack.len();
                        let parent_active = len < 2
                            || cond_stack[len - 2] == CondState::Active;
                        let current = cond_stack[len - 1];
                        match current {
                            CondState::Active => {
                                cond_stack[len - 1] = CondState::Done;
                            }
                            CondState::Inactive => {
                                if parent_active {
                                    let val = self.eval_condition(rest, filename, line_num)?;
                                    if val {
                                        cond_stack[len - 1] = CondState::Active;
                                    }
                                }
                            }
                            CondState::Done => {}
                        }
                    }
                    "else" => {
                        if cond_stack.is_empty() {
                            return Err(PreprocessError::ElseWithoutIf {
                                filename: filename.to_string(),
                                line: line_num,
                            });
                        }
                        let len = cond_stack.len();
                        let parent_active = len < 2
                            || cond_stack[len - 2] == CondState::Active;
                        let current = cond_stack[len - 1];
                        match current {
                            CondState::Active => {
                                cond_stack[len - 1] = CondState::Done;
                            }
                            CondState::Inactive => {
                                if parent_active {
                                    cond_stack[len - 1] = CondState::Active;
                                }
                            }
                            CondState::Done => {}
                        }
                    }
                    "endif" => {
                        if cond_stack.is_empty() {
                            return Err(PreprocessError::EndifWithoutIf {
                                filename: filename.to_string(),
                                line: line_num,
                            });
                        }
                        cond_stack.pop();
                    }
                    "include" => {
                        if !is_active(&cond_stack) {
                            continue;
                        }
                        let included = self.handle_include(rest, filename, line_num)?;
                        output.push_str(&included);
                        output.push('\n');
                    }
                    "error" => {
                        if !is_active(&cond_stack) {
                            continue;
                        }
                        return Err(PreprocessError::UserError {
                            message: rest.to_string(),
                            filename: filename.to_string(),
                            line: line_num,
                        });
                    }
                    "pragma" => {
                        // Ignore pragmas.
                    }
                    "line" => {
                        if !is_active(&cond_stack) {
                            continue;
                        }
                        // Parse #line N "file" - we just update line_num
                        let parts: Vec<&str> = rest.trim().splitn(2, char::is_whitespace).collect();
                        if let Some(num_str) = parts.first() {
                            if let Ok(n) = num_str.parse::<u32>() {
                                line_num = n.saturating_sub(1); // will be incremented next iteration
                            }
                        }
                    }
                    "" => {
                        // Empty directive line (just `#`), ignore.
                    }
                    _ => {
                        if is_active(&cond_stack) {
                            return Err(PreprocessError::UnknownDirective {
                                directive: directive.to_string(),
                                filename: filename.to_string(),
                                line: line_num,
                            });
                        }
                    }
                }
            } else {
                // Non-directive line: emit if active.
                if is_active(&cond_stack) {
                    let expanded = self.expand_macros(raw_line, filename, line_num)?;
                    output.push_str(&expanded);
                    output.push('\n');
                }
            }
        }

        if !cond_stack.is_empty() {
            return Err(PreprocessError::UnterminatedConditional {
                filename: filename.to_string(),
                line: line_num,
            });
        }

        // Remove trailing newline added by line-by-line processing
        if output.ends_with('\n') {
            output.pop();
        }

        Ok(output)
    }

    fn handle_define(&mut self, rest: &str, filename: &str, line: u32) -> Result<(), PreprocessError> {
        let rest = rest.trim();
        if rest.is_empty() {
            return Err(PreprocessError::InvalidMacro {
                filename: filename.to_string(),
                line,
            });
        }

        // Grab the macro name (identifier)
        let name_end = rest
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(rest.len());
        let name = &rest[..name_end];
        if name.is_empty() {
            return Err(PreprocessError::InvalidMacro {
                filename: filename.to_string(),
                line,
            });
        }

        let after_name = &rest[name_end..];

        // Check for parameterized macro: NAME( immediately, no space before paren
        if after_name.starts_with('(') {
            // Find closing paren
            let paren_content_start = 1; // skip '('
            if let Some(close) = after_name.find(')') {
                let params_str = &after_name[paren_content_start..close];
                let params: Vec<String> = if params_str.trim().is_empty() {
                    Vec::new()
                } else {
                    params_str
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .collect()
                };
                let body = after_name[close + 1..].trim().to_string();
                self.defines.insert(
                    name.to_string(),
                    MacroDef::Parameterized { params, body },
                );
            } else {
                return Err(PreprocessError::InvalidMacro {
                    filename: filename.to_string(),
                    line,
                });
            }
        } else {
            // Simple macro
            let value = after_name.trim().to_string();
            self.defines
                .insert(name.to_string(), MacroDef::Simple(value));
        }

        Ok(())
    }

    fn handle_include(
        &mut self,
        rest: &str,
        filename: &str,
        line: u32,
    ) -> Result<String, PreprocessError> {
        let rest = rest.trim();
        let path = if rest.starts_with('"') {
            // #include "file"
            let end = rest[1..].find('"').ok_or_else(|| PreprocessError::IncludeNotFound {
                path: rest.to_string(),
                filename: filename.to_string(),
                line,
            })?;
            &rest[1..1 + end]
        } else if rest.starts_with('<') {
            // #include <file>
            let end = rest[1..].find('>').ok_or_else(|| PreprocessError::IncludeNotFound {
                path: rest.to_string(),
                filename: filename.to_string(),
                line,
            })?;
            &rest[1..1 + end]
        } else {
            return Err(PreprocessError::IncludeNotFound {
                path: rest.to_string(),
                filename: filename.to_string(),
                line,
            });
        };

        let resolver = self.include_resolver.as_ref().ok_or_else(|| {
            PreprocessError::IncludeNotFound {
                path: path.to_string(),
                filename: filename.to_string(),
                line,
            }
        })?;

        let source = resolver(path).ok_or_else(|| PreprocessError::IncludeNotFound {
            path: path.to_string(),
            filename: filename.to_string(),
            line,
        })?;

        // Recursively process the included file.
        self.process(&source, path)
    }

    fn expand_macros(
        &self,
        line_text: &str,
        filename: &str,
        line: u32,
    ) -> Result<String, PreprocessError> {
        let mut result = line_text.to_string();
        // Iterate until no more expansions happen (to handle nested macros).
        // Limit iterations to prevent infinite recursion.
        for _ in 0..64 {
            let mut changed = false;
            for (name, def) in &self.defines {
                match def {
                    MacroDef::Simple(value) => {
                        if let Some(expanded) = replace_identifier(&result, name, value) {
                            result = expanded;
                            changed = true;
                        }
                    }
                    MacroDef::Parameterized { params, body } => {
                        if let Some(expanded) =
                            expand_parameterized(&result, name, params, body, filename, line)?
                        {
                            result = expanded;
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        Ok(result)
    }

    fn eval_condition(
        &self,
        expr_str: &str,
        filename: &str,
        line: u32,
    ) -> Result<bool, PreprocessError> {
        // Process defined() BEFORE macro expansion so that `defined(FOO)`
        // checks the symbol table before FOO gets expanded to its value.
        let with_defined = self.resolve_defined(expr_str);
        // Then expand remaining macros.
        let expanded = self.expand_macros(&with_defined, filename, line)?;
        // Evaluate the expression.
        let mut evaluator = CondExprEvaluator::new(&expanded, &self.defines, filename, line);
        let val = evaluator.parse_expr()?;
        Ok(val != 0)
    }

    /// Replace `defined(NAME)` and `defined NAME` with `1` or `0` before
    /// macro expansion runs. This prevents the macro value from replacing
    /// the identifier inside `defined()`.
    fn resolve_defined(&self, expr: &str) -> String {
        let mut result = String::new();
        let chars: Vec<char> = expr.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            // Look for "defined" keyword at word boundary
            if i + 7 <= chars.len() && &expr[i..i + 7] == "defined" {
                // Check word boundary before
                let before_ok = i == 0 || {
                    let prev = chars[i - 1];
                    !prev.is_ascii_alphanumeric() && prev != '_'
                };
                // Check word boundary after
                let after_pos = i + 7;
                let after_ok = after_pos >= chars.len() || {
                    let next = chars[after_pos];
                    !next.is_ascii_alphanumeric() && next != '_'
                };
                if before_ok && after_ok {
                    let mut j = after_pos;
                    // Skip whitespace
                    while j < chars.len() && chars[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    let has_paren = j < chars.len() && chars[j] == '(';
                    if has_paren {
                        j += 1;
                    }
                    // Skip whitespace
                    while j < chars.len() && chars[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    // Parse identifier
                    let ident_start = j;
                    while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_')
                    {
                        j += 1;
                    }
                    let name: String = chars[ident_start..j].iter().collect();
                    if !name.is_empty() {
                        if has_paren {
                            // Skip whitespace and closing paren
                            while j < chars.len() && chars[j].is_ascii_whitespace() {
                                j += 1;
                            }
                            if j < chars.len() && chars[j] == ')' {
                                j += 1;
                            }
                        }
                        let val = if self.defines.contains_key(&name) {
                            "1"
                        } else {
                            "0"
                        };
                        result.push_str(val);
                        i = j;
                        continue;
                    }
                }
            }
            result.push(chars[i]);
            i += 1;
        }
        result
    }
}

impl Default for Preprocessor {
    fn default() -> Self {
        Self::new()
    }
}

// ---- Helper functions ----

/// Join lines that end with `\` (line continuation).
fn join_line_continuations(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && chars.peek() == Some(&'\n') {
            chars.next(); // consume the newline
            continue;
        }
        result.push(ch);
    }
    result
}

fn split_directive(text: &str) -> (&str, &str) {
    let text = text.trim();
    if let Some(idx) = text.find(|c: char| c.is_ascii_whitespace()) {
        (&text[..idx], text[idx..].trim_start())
    } else {
        (text, "")
    }
}

fn is_active(stack: &[CondState]) -> bool {
    stack.iter().all(|s| *s == CondState::Active)
}

/// Replace whole-word occurrences of `name` with `replacement` in `text`.
/// Returns None if no replacement was made.
fn replace_identifier(text: &str, name: &str, replacement: &str) -> Option<String> {
    let mut result = String::new();
    let mut rest = text;
    let mut changed = false;

    while let Some(pos) = rest.find(name) {
        // Check word boundary before
        let before_ok = if pos == 0 {
            true
        } else {
            let prev = rest.as_bytes()[pos - 1] as char;
            !prev.is_ascii_alphanumeric() && prev != '_'
        };
        // Check word boundary after
        let after_pos = pos + name.len();
        let after_ok = if after_pos >= rest.len() {
            true
        } else {
            let next = rest.as_bytes()[after_pos] as char;
            !next.is_ascii_alphanumeric() && next != '_'
        };

        if before_ok && after_ok {
            result.push_str(&rest[..pos]);
            result.push_str(replacement);
            rest = &rest[after_pos..];
            changed = true;
        } else {
            result.push_str(&rest[..after_pos]);
            rest = &rest[after_pos..];
        }
    }

    if changed {
        result.push_str(rest);
        Some(result)
    } else {
        None
    }
}

/// Expand a parameterized macro invocation. Returns None if no invocation found.
fn expand_parameterized(
    text: &str,
    name: &str,
    params: &[String],
    body: &str,
    filename: &str,
    line: u32,
) -> Result<Option<String>, PreprocessError> {
    // Find NAME( pattern with word boundary
    let pattern = format!("{}(", name);
    let Some(start) = text.find(&pattern) else {
        return Ok(None);
    };

    // Verify word boundary before
    if start > 0 {
        let prev = text.as_bytes()[start - 1] as char;
        if prev.is_ascii_alphanumeric() || prev == '_' {
            return Ok(None);
        }
    }

    let args_start = start + name.len() + 1; // after the '('

    // Parse arguments, handling nested parens
    let mut args = Vec::new();
    let mut depth = 1;
    let mut current_arg = String::new();
    let mut i = args_start;
    let bytes = text.as_bytes();

    while i < bytes.len() && depth > 0 {
        let ch = bytes[i] as char;
        match ch {
            '(' => {
                depth += 1;
                current_arg.push(ch);
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    args.push(current_arg.trim().to_string());
                } else {
                    current_arg.push(ch);
                }
            }
            ',' if depth == 1 => {
                args.push(current_arg.trim().to_string());
                current_arg = String::new();
            }
            _ => {
                current_arg.push(ch);
            }
        }
        i += 1;
    }

    if depth != 0 {
        return Err(PreprocessError::MacroArgMismatch {
            name: name.to_string(),
            filename: filename.to_string(),
            line,
        });
    }

    // Handle the case of zero-parameter macros called with empty parens
    if params.is_empty() && args.len() == 1 && args[0].is_empty() {
        args.clear();
    }

    if args.len() != params.len() {
        return Err(PreprocessError::MacroArgMismatch {
            name: name.to_string(),
            filename: filename.to_string(),
            line,
        });
    }

    // Substitute parameters in body
    let mut expanded_body = body.to_string();
    for (param, arg) in params.iter().zip(args.iter()) {
        if let Some(replaced) = replace_identifier(&expanded_body, param, arg) {
            expanded_body = replaced;
        }
    }

    let mut result = String::new();
    result.push_str(&text[..start]);
    result.push_str(&expanded_body);
    result.push_str(&text[i..]);

    Ok(Some(result))
}

// ---- Conditional expression evaluator ----
// Supports integer arithmetic for #if / #elif expressions.

struct CondExprEvaluator<'a> {
    input: Vec<char>,
    pos: usize,
    defines: &'a HashMap<String, MacroDef>,
    filename: &'a str,
    line: u32,
}

impl<'a> CondExprEvaluator<'a> {
    fn new(
        expr: &str,
        defines: &'a HashMap<String, MacroDef>,
        filename: &'a str,
        line: u32,
    ) -> Self {
        Self {
            input: expr.chars().collect(),
            pos: 0,
            defines,
            filename,
            line,
        }
    }

    fn error(&self, detail: &str) -> PreprocessError {
        PreprocessError::InvalidExpression {
            detail: detail.to_string(),
            filename: self.filename.to_string(),
            line: self.line,
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.input.get(self.pos).copied();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }

    fn parse_expr(&mut self) -> Result<i64, PreprocessError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<i64, PreprocessError> {
        let mut left = self.parse_and()?;
        loop {
            self.skip_ws();
            if self.pos + 1 < self.input.len()
                && self.input[self.pos] == '|'
                && self.input[self.pos + 1] == '|'
            {
                self.pos += 2;
                let right = self.parse_and()?;
                left = if left != 0 || right != 0 { 1 } else { 0 };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<i64, PreprocessError> {
        let mut left = self.parse_equality()?;
        loop {
            self.skip_ws();
            if self.pos + 1 < self.input.len()
                && self.input[self.pos] == '&'
                && self.input[self.pos + 1] == '&'
            {
                self.pos += 2;
                let right = self.parse_equality()?;
                left = if left != 0 && right != 0 { 1 } else { 0 };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<i64, PreprocessError> {
        let mut left = self.parse_relational()?;
        loop {
            self.skip_ws();
            if self.pos + 1 < self.input.len()
                && self.input[self.pos] == '='
                && self.input[self.pos + 1] == '='
            {
                self.pos += 2;
                let right = self.parse_relational()?;
                left = if left == right { 1 } else { 0 };
            } else if self.pos + 1 < self.input.len()
                && self.input[self.pos] == '!'
                && self.input[self.pos + 1] == '='
            {
                self.pos += 2;
                let right = self.parse_relational()?;
                left = if left != right { 1 } else { 0 };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_relational(&mut self) -> Result<i64, PreprocessError> {
        let mut left = self.parse_additive()?;
        loop {
            self.skip_ws();
            if self.pos + 1 < self.input.len()
                && self.input[self.pos] == '<'
                && self.input[self.pos + 1] == '='
            {
                self.pos += 2;
                let right = self.parse_additive()?;
                left = if left <= right { 1 } else { 0 };
            } else if self.pos + 1 < self.input.len()
                && self.input[self.pos] == '>'
                && self.input[self.pos + 1] == '='
            {
                self.pos += 2;
                let right = self.parse_additive()?;
                left = if left >= right { 1 } else { 0 };
            } else if self.pos < self.input.len() && self.input[self.pos] == '<' {
                // Make sure it's not <=
                if self.pos + 1 < self.input.len() && self.input[self.pos + 1] == '=' {
                    break;
                }
                self.pos += 1;
                let right = self.parse_additive()?;
                left = if left < right { 1 } else { 0 };
            } else if self.pos < self.input.len() && self.input[self.pos] == '>' {
                if self.pos + 1 < self.input.len() && self.input[self.pos + 1] == '=' {
                    break;
                }
                self.pos += 1;
                let right = self.parse_additive()?;
                left = if left > right { 1 } else { 0 };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<i64, PreprocessError> {
        let mut left = self.parse_multiplicative()?;
        loop {
            self.skip_ws();
            if self.pos < self.input.len() && self.input[self.pos] == '+' {
                self.pos += 1;
                let right = self.parse_multiplicative()?;
                left = left.wrapping_add(right);
            } else if self.pos < self.input.len() && self.input[self.pos] == '-' {
                self.pos += 1;
                let right = self.parse_multiplicative()?;
                left = left.wrapping_sub(right);
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<i64, PreprocessError> {
        let mut left = self.parse_unary()?;
        loop {
            self.skip_ws();
            if self.pos < self.input.len() && self.input[self.pos] == '*' {
                self.pos += 1;
                let right = self.parse_unary()?;
                left = left.wrapping_mul(right);
            } else if self.pos < self.input.len() && self.input[self.pos] == '/' {
                self.pos += 1;
                let right = self.parse_unary()?;
                if right == 0 {
                    return Err(self.error("division by zero"));
                }
                left = left.wrapping_div(right);
            } else if self.pos < self.input.len() && self.input[self.pos] == '%' {
                self.pos += 1;
                let right = self.parse_unary()?;
                if right == 0 {
                    return Err(self.error("division by zero"));
                }
                left = left.wrapping_rem(right);
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<i64, PreprocessError> {
        self.skip_ws();
        if let Some(ch) = self.peek() {
            if ch == '!' {
                self.advance();
                let val = self.parse_unary()?;
                return Ok(if val == 0 { 1 } else { 0 });
            }
            if ch == '-' {
                self.advance();
                let val = self.parse_unary()?;
                return Ok(-val);
            }
            if ch == '+' {
                self.advance();
                return self.parse_unary();
            }
            if ch == '~' {
                self.advance();
                let val = self.parse_unary()?;
                return Ok(!val);
            }
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<i64, PreprocessError> {
        self.skip_ws();
        let ch = self.peek().ok_or_else(|| self.error("unexpected end of expression"))?;

        // Parenthesized expression
        if ch == '(' {
            self.advance();
            let val = self.parse_expr()?;
            self.skip_ws();
            if self.peek() != Some(')') {
                return Err(self.error("expected ')'"));
            }
            self.advance();
            return Ok(val);
        }

        // Number literal
        if ch.is_ascii_digit() {
            return self.parse_number();
        }

        // Identifier: could be `defined(NAME)` or an undefined macro (treat as 0)
        if ch.is_ascii_alphabetic() || ch == '_' {
            let ident = self.parse_ident();
            if ident == "defined" {
                return self.parse_defined();
            }
            // Unknown identifier in #if: treat as 0 (standard C preprocessor behavior)
            return Ok(0);
        }

        Err(self.error(&format!("unexpected character '{ch}'")))
    }

    fn parse_number(&mut self) -> Result<i64, PreprocessError> {
        let mut s = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_ascii_hexdigit() || ch == 'x' || ch == 'X' {
                s.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        if s.starts_with("0x") || s.starts_with("0X") {
            i64::from_str_radix(&s[2..], 16).map_err(|_| self.error("invalid hex number"))
        } else if s.starts_with('0') && s.len() > 1 {
            i64::from_str_radix(&s[1..], 8).map_err(|_| self.error("invalid octal number"))
        } else {
            s.parse().map_err(|_| self.error("invalid number"))
        }
    }

    fn parse_ident(&mut self) -> String {
        let mut s = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                s.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        s
    }

    fn parse_defined(&mut self) -> Result<i64, PreprocessError> {
        self.skip_ws();
        let has_paren = self.peek() == Some('(');
        if has_paren {
            self.advance();
        }
        self.skip_ws();
        let name = self.parse_ident();
        if name.is_empty() {
            return Err(self.error("expected identifier after 'defined'"));
        }
        if has_paren {
            self.skip_ws();
            if self.peek() != Some(')') {
                return Err(self.error("expected ')' after defined(NAME"));
            }
            self.advance();
        }
        Ok(if self.defines.contains_key(&name) { 1 } else { 0 })
    }
}
