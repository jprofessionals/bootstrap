//! Object serialization kfuns: save_object, restore_object.
//!
//! Format: one variable per line, "varname value\n".
//! Values are serialized in LPC literal format:
//! - nil -> "nil"
//! - int -> decimal number
//! - float -> decimal with 6 decimal places
//! - string -> quoted with escapes
//! - array -> ({elem,elem,...})
//! - mapping -> ([key:val,key:val,...])
//! - object -> <path>

use crate::bytecode::LpcValue;
use super::{KfunContext, LpcError};

/// save_object(string path) -> void
///
/// Serialize the current object's variables to a file.
///
/// TODO: Full implementation requires VM variable access and driver services
/// for file I/O.
pub fn kf_save_object(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    // TODO: Requires VM variable access + driver services for file I/O
    Err(LpcError::RuntimeError(
        "save_object: driver services not yet connected".into(),
    ))
}

/// restore_object(string path) -> void
///
/// Read back serialized variables and set them on the current object.
///
/// TODO: Full implementation requires VM variable access and driver services
/// for file I/O.
pub fn kf_restore_object(
    _ctx: &mut KfunContext,
    _args: &[LpcValue],
) -> Result<LpcValue, LpcError> {
    // TODO: Requires VM variable access + driver services for file I/O
    Err(LpcError::RuntimeError(
        "restore_object: driver services not yet connected".into(),
    ))
}

/// Serialize a list of (name, value) pairs to the save_object text format.
pub fn serialize_variables(vars: &[(String, LpcValue)]) -> String {
    let mut out = String::new();
    for (name, value) in vars {
        out.push_str(name);
        out.push(' ');
        serialize_value(&mut out, value);
        out.push('\n');
    }
    out
}

/// Serialize a single LpcValue to its text representation.
pub fn serialize_value(out: &mut String, value: &LpcValue) {
    match value {
        LpcValue::Nil => out.push_str("nil"),
        LpcValue::Int(n) => out.push_str(&n.to_string()),
        LpcValue::Float(f) => out.push_str(&format!("{:.6}", f)),
        LpcValue::String(s) => {
            out.push('"');
            for ch in s.chars() {
                match ch {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\t' => out.push_str("\\t"),
                    '\r' => out.push_str("\\r"),
                    _ => out.push(ch),
                }
            }
            out.push('"');
        }
        LpcValue::Array(arr) => {
            out.push_str("({");
            for (i, v) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                serialize_value(out, v);
            }
            out.push_str("})");
        }
        LpcValue::Mapping(m) => {
            out.push_str("([");
            for (i, (k, v)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                serialize_value(out, k);
                out.push(':');
                serialize_value(out, v);
            }
            out.push_str("])");
        }
        LpcValue::Object(obj) => {
            out.push('<');
            out.push_str(&obj.path);
            out.push('>');
        }
    }
}

/// Parse a serialized save file back into (name, value) pairs.
pub fn parse_saved_variables(content: &str) -> Result<Vec<(String, LpcValue)>, LpcError> {
    let mut result = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Find the first space separating name from value
        let space_pos = line
            .find(' ')
            .ok_or_else(|| LpcError::ValueError(format!("invalid save line: {}", line)))?;
        let name = line[..space_pos].to_string();
        let value_str = &line[space_pos + 1..];
        let (value, _) = parse_value(value_str)?;
        result.push((name, value));
    }
    Ok(result)
}

/// Parse a single value from a string, returning the value and remaining input.
pub fn parse_value(input: &str) -> Result<(LpcValue, &str), LpcError> {
    let input = input.trim_start();
    if input.is_empty() {
        return Ok((LpcValue::Nil, ""));
    }

    // nil
    if input.starts_with("nil") {
        return Ok((LpcValue::Nil, &input[3..]));
    }

    // String
    if input.starts_with('"') {
        return parse_string_value(input);
    }

    // Array
    if input.starts_with("({") {
        return parse_array_value(input);
    }

    // Mapping
    if input.starts_with("([") {
        return parse_mapping_value(input);
    }

    // Object reference
    if input.starts_with('<') {
        if let Some(end) = input.find('>') {
            let path = input[1..end].to_string();
            return Ok((
                LpcValue::Object(crate::bytecode::ObjectRef {
                    id: 0,
                    path,
                    is_lightweight: false,
                }),
                &input[end + 1..],
            ));
        }
        return Err(LpcError::ValueError(
            "unterminated object reference".into(),
        ));
    }

    // Float (contains a dot)
    // Int (digits, possibly with leading sign)
    if input.starts_with('-')
        || input.starts_with('+')
        || input.as_bytes().first().map_or(false, |b| b.is_ascii_digit())
    {
        // Find end of number
        let mut end = 0;
        let bytes = input.as_bytes();
        if end < bytes.len() && (bytes[end] == b'-' || bytes[end] == b'+') {
            end += 1;
        }
        let mut is_float = false;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end < bytes.len() && bytes[end] == b'.' {
            is_float = true;
            end += 1;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
        }
        // Scientific notation
        if end < bytes.len() && (bytes[end] == b'e' || bytes[end] == b'E') {
            is_float = true;
            end += 1;
            if end < bytes.len() && (bytes[end] == b'-' || bytes[end] == b'+') {
                end += 1;
            }
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
        }

        let num_str = &input[..end];
        let rest = &input[end..];

        if is_float {
            let f: f64 = num_str
                .parse()
                .map_err(|_| LpcError::ValueError(format!("invalid float: {}", num_str)))?;
            return Ok((LpcValue::Float(f), rest));
        } else {
            let n: i64 = num_str
                .parse()
                .map_err(|_| LpcError::ValueError(format!("invalid int: {}", num_str)))?;
            return Ok((LpcValue::Int(n), rest));
        }
    }

    Err(LpcError::ValueError(format!(
        "cannot parse value: {}",
        &input[..input.len().min(20)]
    )))
}

fn parse_string_value(input: &str) -> Result<(LpcValue, &str), LpcError> {
    debug_assert!(input.starts_with('"'));
    let bytes = input.as_bytes();
    let mut i = 1;
    let mut s = String::new();

    while i < bytes.len() {
        if bytes[i] == b'"' {
            return Ok((LpcValue::String(s), &input[i + 1..]));
        }
        if bytes[i] == b'\\' {
            i += 1;
            if i >= bytes.len() {
                return Err(LpcError::ValueError("unterminated string escape".into()));
            }
            match bytes[i] {
                b'n' => s.push('\n'),
                b't' => s.push('\t'),
                b'r' => s.push('\r'),
                b'\\' => s.push('\\'),
                b'"' => s.push('"'),
                other => {
                    s.push('\\');
                    s.push(other as char);
                }
            }
        } else {
            s.push(bytes[i] as char);
        }
        i += 1;
    }

    Err(LpcError::ValueError("unterminated string".into()))
}

fn parse_array_value(input: &str) -> Result<(LpcValue, &str), LpcError> {
    debug_assert!(input.starts_with("({"));
    let mut rest = &input[2..];
    let mut elements = Vec::new();

    rest = rest.trim_start();
    if rest.starts_with("})") {
        return Ok((LpcValue::Array(elements), &rest[2..]));
    }

    loop {
        let (val, remaining) = parse_value(rest)?;
        elements.push(val);
        rest = remaining.trim_start();

        if rest.starts_with("})") {
            return Ok((LpcValue::Array(elements), &rest[2..]));
        }
        if rest.starts_with(',') {
            rest = &rest[1..];
        } else {
            return Err(LpcError::ValueError(
                "expected ',' or '})' in array".into(),
            ));
        }
    }
}

fn parse_mapping_value(input: &str) -> Result<(LpcValue, &str), LpcError> {
    debug_assert!(input.starts_with("(["));
    let mut rest = &input[2..];
    let mut pairs = Vec::new();

    rest = rest.trim_start();
    if rest.starts_with("])") {
        return Ok((LpcValue::Mapping(pairs), &rest[2..]));
    }

    loop {
        let (key, remaining) = parse_value(rest)?;
        rest = remaining.trim_start();

        if !rest.starts_with(':') {
            return Err(LpcError::ValueError(
                "expected ':' in mapping".into(),
            ));
        }
        rest = &rest[1..];

        let (val, remaining) = parse_value(rest)?;
        pairs.push((key, val));
        rest = remaining.trim_start();

        if rest.starts_with("])") {
            return Ok((LpcValue::Mapping(pairs), &rest[2..]));
        }
        if rest.starts_with(',') {
            rest = &rest[1..];
        } else {
            return Err(LpcError::ValueError(
                "expected ',' or '])' in mapping".into(),
            ));
        }
    }
}
