//! String operation kfuns: strlen, explode, implode, lower_case, upper_case, sscanf.

use super::{require_array, require_string, KfunContext, LpcError};
use crate::bytecode::LpcValue;

/// strlen(string s) -> int
///
/// Returns the length of a string.
pub fn kf_strlen(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s = require_string(&args[0], 0)?;
    Ok(LpcValue::Int(s.len() as i64))
}

/// explode(string s, string separator) -> string*
///
/// Split string by separator. If separator is empty, splits into individual
/// characters. Leading/trailing separators do NOT produce empty strings (DGD behavior).
pub fn kf_explode(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s = require_string(&args[0], 0)?;
    let sep = require_string(&args[1], 1)?;

    let parts: Vec<LpcValue> = if sep.is_empty() {
        // Split into individual characters
        s.chars().map(|c| LpcValue::String(c.to_string())).collect()
    } else {
        // Split by separator, filter out empty strings at edges (DGD behavior)
        s.split(sep)
            .filter(|p| !p.is_empty())
            .map(|p| LpcValue::String(p.to_string()))
            .collect()
    };

    *ctx.tick_counter = ctx.tick_counter.saturating_sub(parts.len() as u64);
    Ok(LpcValue::Array(parts))
}

/// implode(string* arr, string separator) -> string
///
/// Join array of strings with separator. Error if any element is not a string.
pub fn kf_implode(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let arr = require_array(&args[0], 0)?;
    let sep = require_string(&args[1], 1)?;

    let mut parts = Vec::with_capacity(arr.len());
    for (i, v) in arr.iter().enumerate() {
        match v {
            LpcValue::String(s) => parts.push(s.as_str()),
            _ => {
                return Err(LpcError::TypeError {
                    expected: "string",
                    got: v.type_name().to_string(),
                    arg_pos: i,
                })
            }
        }
    }

    *ctx.tick_counter = ctx.tick_counter.saturating_sub(arr.len() as u64);
    Ok(LpcValue::String(parts.join(sep)))
}

/// lower_case(string s) -> string
///
/// Convert string to lowercase.
pub fn kf_lower_case(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s = require_string(&args[0], 0)?;
    *ctx.tick_counter = ctx.tick_counter.saturating_sub(s.len() as u64 / 2);
    Ok(LpcValue::String(s.to_lowercase()))
}

/// upper_case(string s) -> string
///
/// Convert string to uppercase.
pub fn kf_upper_case(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s = require_string(&args[0], 0)?;
    *ctx.tick_counter = ctx.tick_counter.saturating_sub(s.len() as u64 / 2);
    Ok(LpcValue::String(s.to_uppercase()))
}

/// sscanf(string input, string format, args...) -> int
///
/// Formatted string scanning. Format specifiers:
/// - %s -- match string (greedy, up to next literal or format)
/// - %d -- match integer
/// - %f -- match float
/// - %c -- match single character
/// - %*s, %*d, etc. -- match but don't assign
///
/// Returns number of successful matches (not counting %* specifiers).
/// Note: In a full VM, matched values would be assigned to lvalue arguments.
/// This implementation returns the match count and stores matched values in
/// an array (the third argument onward would be lvalue references in the VM).
pub fn kf_sscanf(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let input = require_string(&args[0], 0)?;
    let format = require_string(&args[1], 1)?;

    // Parse format string into segments
    let segments = parse_format(format)?;

    let mut pos = 0;
    let mut match_count: i64 = 0;

    for (i, seg) in segments.iter().enumerate() {
        if pos > input.len() {
            break;
        }
        match seg {
            FormatSegment::Literal(lit) => {
                if input[pos..].starts_with(lit.as_str()) {
                    pos += lit.len();
                } else {
                    break;
                }
            }
            FormatSegment::MatchInt { suppress } => {
                // Match optional sign + digits
                let remaining = &input[pos..];
                let mut end = 0;
                if end < remaining.len()
                    && (remaining.as_bytes()[end] == b'-' || remaining.as_bytes()[end] == b'+')
                {
                    end += 1;
                }
                while end < remaining.len() && remaining.as_bytes()[end].is_ascii_digit() {
                    end += 1;
                }
                if end == 0 || (end == 1 && !remaining.as_bytes()[0].is_ascii_digit()) {
                    break;
                }
                pos += end;
                if !suppress {
                    match_count += 1;
                }
                *ctx.tick_counter = ctx.tick_counter.saturating_sub(8);
            }
            FormatSegment::MatchFloat { suppress } => {
                let remaining = &input[pos..];
                let mut end = 0;
                // Optional sign
                if end < remaining.len()
                    && (remaining.as_bytes()[end] == b'-' || remaining.as_bytes()[end] == b'+')
                {
                    end += 1;
                }
                // Digits before decimal
                while end < remaining.len() && remaining.as_bytes()[end].is_ascii_digit() {
                    end += 1;
                }
                // Decimal point
                if end < remaining.len() && remaining.as_bytes()[end] == b'.' {
                    end += 1;
                    while end < remaining.len() && remaining.as_bytes()[end].is_ascii_digit() {
                        end += 1;
                    }
                }
                if end == 0 {
                    break;
                }
                pos += end;
                if !suppress {
                    match_count += 1;
                }
                *ctx.tick_counter = ctx.tick_counter.saturating_sub(8);
            }
            FormatSegment::MatchChar { suppress } => {
                if pos < input.len() {
                    // Advance by one character (UTF-8 aware)
                    let ch_len = input[pos..]
                        .chars()
                        .next()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                    if ch_len == 0 {
                        break;
                    }
                    pos += ch_len;
                    if !suppress {
                        match_count += 1;
                    }
                    *ctx.tick_counter = ctx.tick_counter.saturating_sub(8);
                } else {
                    break;
                }
            }
            FormatSegment::MatchString { suppress } => {
                // %s: greedy match up to next literal or end
                let next_seg = segments.get(i + 1);
                let end_pos = match next_seg {
                    Some(FormatSegment::Literal(lit)) => {
                        // Find the next occurrence of the literal
                        input[pos..].find(lit.as_str()).map(|p| pos + p)
                    }
                    Some(FormatSegment::MatchInt { .. }) => {
                        // Match up to first digit (or sign+digit)
                        let remaining = &input[pos..];
                        let mut found = None;
                        for (idx, b) in remaining.bytes().enumerate() {
                            if b.is_ascii_digit() || b == b'-' || b == b'+' {
                                // Check if it's actually the start of a number
                                if b.is_ascii_digit() {
                                    found = Some(pos + idx);
                                    break;
                                }
                                // Sign followed by digit
                                if idx + 1 < remaining.len()
                                    && remaining.as_bytes()[idx + 1].is_ascii_digit()
                                {
                                    found = Some(pos + idx);
                                    break;
                                }
                            }
                        }
                        found
                    }
                    None => {
                        // %s at end: match rest of string
                        Some(input.len())
                    }
                    _ => Some(input.len()),
                };

                match end_pos {
                    Some(ep) => {
                        pos = ep;
                        if !suppress {
                            match_count += 1;
                        }
                        *ctx.tick_counter = ctx.tick_counter.saturating_sub(8);
                    }
                    None => break,
                }
            }
        }
    }

    Ok(LpcValue::Int(match_count))
}

/// Internal format segment for sscanf parsing.
enum FormatSegment {
    Literal(String),
    MatchString { suppress: bool },
    MatchInt { suppress: bool },
    MatchFloat { suppress: bool },
    MatchChar { suppress: bool },
}

/// Parse a sscanf format string into segments.
fn parse_format(format: &str) -> Result<Vec<FormatSegment>, LpcError> {
    let mut segments = Vec::new();
    let bytes = format.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' {
            i += 1;
            if i >= bytes.len() {
                return Err(LpcError::ValueError(
                    "incomplete format specifier".to_string(),
                ));
            }
            let suppress = if bytes[i] == b'*' {
                i += 1;
                if i >= bytes.len() {
                    return Err(LpcError::ValueError(
                        "incomplete format specifier".to_string(),
                    ));
                }
                true
            } else {
                false
            };
            match bytes[i] {
                b's' => {
                    segments.push(FormatSegment::MatchString { suppress });
                    i += 1;
                }
                b'd' => {
                    segments.push(FormatSegment::MatchInt { suppress });
                    i += 1;
                }
                b'f' => {
                    segments.push(FormatSegment::MatchFloat { suppress });
                    i += 1;
                }
                b'c' => {
                    segments.push(FormatSegment::MatchChar { suppress });
                    i += 1;
                }
                b'%' => {
                    // Literal %
                    segments.push(FormatSegment::Literal("%".to_string()));
                    i += 1;
                }
                other => {
                    return Err(LpcError::ValueError(format!(
                        "unknown format specifier: %{}",
                        other as char
                    )));
                }
            }
        } else {
            // Literal text
            let start = i;
            while i < bytes.len() && bytes[i] != b'%' {
                i += 1;
            }
            segments.push(FormatSegment::Literal(
                std::str::from_utf8(&bytes[start..i])
                    .unwrap_or("")
                    .to_string(),
            ));
        }
    }

    Ok(segments)
}
