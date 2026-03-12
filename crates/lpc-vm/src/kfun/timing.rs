//! Timing and scheduling kfuns: time, millitime, ctime, call_out, remove_call_out,
//! call_out_summand.

use std::time::SystemTime;

use super::{require_int, require_string, KfunContext, LpcError};
use crate::bytecode::LpcValue;

/// time() -> int
///
/// Returns current Unix timestamp (seconds since epoch).
pub fn kf_time(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    Ok(LpcValue::Int(now))
}

/// millitime() -> mixed*
///
/// Returns ({seconds, millisecond_fraction}) -- the seconds as int and the
/// sub-second fraction as float (0.0 to 0.999...).
pub fn kf_millitime(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let secs = now.as_secs() as i64;
    let frac = (now.subsec_millis() as f64) / 1000.0;
    Ok(LpcValue::Array(vec![
        LpcValue::Int(secs),
        LpcValue::Float(frac),
    ]))
}

/// ctime(int timestamp) -> string
///
/// Convert Unix timestamp to human-readable date string.
/// Returns a string in the format: "Ddd Mmm DD HH:MM:SS YYYY"
///
/// Manual implementation without chrono dependency. Uses a simplified
/// algorithm to format the timestamp.
pub fn kf_ctime(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let ts = require_int(&args[0], 0)?;

    let (year, month, day, hour, min, sec, wday) = unix_timestamp_to_parts(ts);

    let day_names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let month_names = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    let formatted = format!(
        "{} {} {:2} {:02}:{:02}:{:02} {:04}",
        day_names[wday as usize],
        month_names[(month - 1) as usize],
        day,
        hour,
        min,
        sec,
        year
    );

    Ok(LpcValue::String(formatted))
}

/// Break a Unix timestamp into (year, month, day, hour, minute, second, weekday).
fn unix_timestamp_to_parts(ts: i64) -> (i64, i64, i64, i64, i64, i64, i64) {
    let sec = ts % 60;
    let min = (ts / 60) % 60;
    let hour = (ts / 3600) % 24;

    // Days since epoch (Jan 1, 1970)
    let mut days = ts / 86400;
    // Jan 1, 1970 was a Thursday (day 4)
    let wday = ((days % 7) + 4) % 7;

    // Adjust for negative timestamps
    let mut year = 1970i64;
    if days >= 0 {
        loop {
            let days_in_year = if is_leap_year(year) { 366 } else { 365 };
            if days < days_in_year {
                break;
            }
            days -= days_in_year;
            year += 1;
        }
    } else {
        loop {
            year -= 1;
            let days_in_year = if is_leap_year(year) { 366 } else { 365 };
            days += days_in_year;
            if days >= 0 {
                break;
            }
        }
    }

    let leap = is_leap_year(year);
    let month_days: [i64; 12] = if leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1i64;
    for &md in &month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }

    let day = days + 1;

    (year, month, day, hour, min, sec, wday)
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// call_out(string func, mixed delay, args...) -> int
///
/// Schedule a function call after a delay. Returns a handle for cancellation.
///
/// TODO: Full implementation requires scheduler integration.
pub fn kf_call_out(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let _func = require_string(&args[0], 0)?;
    // Validate the delay argument type
    match &args[1] {
        LpcValue::Int(_) | LpcValue::Float(_) => {}
        _ => {
            return Err(LpcError::TypeError {
                expected: "int or float",
                got: args[1].type_name().to_string(),
                arg_pos: 1,
            })
        }
    }
    // TODO: Requires scheduler integration
    Err(LpcError::RuntimeError(
        "call_out: scheduler not yet connected".into(),
    ))
}

/// remove_call_out(int handle) -> mixed
///
/// Cancel a pending call_out. Returns remaining delay, or nil if not found.
///
/// TODO: Full implementation requires scheduler integration.
pub fn kf_remove_call_out(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let _handle = require_int(&args[0], 0)?;
    // TODO: Requires scheduler integration
    Err(LpcError::RuntimeError(
        "remove_call_out: scheduler not yet connected".into(),
    ))
}

/// call_out_summand(string func, mixed delay, args...) -> int
///
/// Schedule a delayed call with summand accumulation. Similar to call_out,
/// but if the same function is already scheduled, the delay is added to
/// the existing call_out rather than creating a new one.
pub fn kf_call_out_summand(
    _ctx: &mut KfunContext,
    _args: &[LpcValue],
) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "call_out_summand: driver services not yet connected".into(),
    ))
}
