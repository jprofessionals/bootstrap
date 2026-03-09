//! Math operation kfuns: fabs, floor, ceil, sqrt, exp, log, log10, sin, cos,
//! tan, asin, acos, atan, sinh, cosh, tanh, pow, fmod, atan2, ldexp, frexp,
//! modf, random.

use rand::Rng;

use crate::bytecode::LpcValue;
use super::{KfunContext, LpcError, require_float, require_int};

/// Helper: apply a single-argument float function with NaN/Inf checking.
fn float_unary(
    _ctx: &mut KfunContext,
    args: &[LpcValue],
    f: fn(f64) -> f64,
) -> Result<LpcValue, LpcError> {
    let x = require_float(&args[0], 0)?;
    let result = f(x);
    if result.is_nan() || result.is_infinite() {
        return Err(LpcError::ValueError("math domain error".into()));
    }
    Ok(LpcValue::Float(result))
}

/// fabs(float x) -> float
pub fn kf_fabs(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    float_unary(ctx, args, f64::abs)
}

/// floor(float x) -> float
pub fn kf_floor(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    float_unary(ctx, args, f64::floor)
}

/// ceil(float x) -> float
pub fn kf_ceil(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    float_unary(ctx, args, f64::ceil)
}

/// sqrt(float x) -> float
///
/// Error if x is negative.
pub fn kf_sqrt(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let x = require_float(&args[0], 0)?;
    if x < 0.0 {
        return Err(LpcError::ValueError(
            "math domain error: sqrt of negative".into(),
        ));
    }
    let _ = ctx;
    let result = x.sqrt();
    Ok(LpcValue::Float(result))
}

/// exp(float x) -> float
pub fn kf_exp(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    float_unary(ctx, args, f64::exp)
}

/// log(float x) -> float
///
/// Natural logarithm. Error if x <= 0.
pub fn kf_log(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let x = require_float(&args[0], 0)?;
    if x <= 0.0 {
        return Err(LpcError::ValueError(
            "math domain error: log of non-positive".into(),
        ));
    }
    Ok(LpcValue::Float(x.ln()))
}

/// log10(float x) -> float
///
/// Base-10 logarithm. Error if x <= 0.
pub fn kf_log10(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let x = require_float(&args[0], 0)?;
    if x <= 0.0 {
        return Err(LpcError::ValueError(
            "math domain error: log10 of non-positive".into(),
        ));
    }
    Ok(LpcValue::Float(x.log10()))
}

/// sin(float x) -> float
pub fn kf_sin(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    float_unary(ctx, args, f64::sin)
}

/// cos(float x) -> float
pub fn kf_cos(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    float_unary(ctx, args, f64::cos)
}

/// tan(float x) -> float
pub fn kf_tan(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    float_unary(ctx, args, f64::tan)
}

/// asin(float x) -> float
///
/// Error if |x| > 1.
pub fn kf_asin(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let x = require_float(&args[0], 0)?;
    if x < -1.0 || x > 1.0 {
        return Err(LpcError::ValueError(
            "math domain error: asin argument out of range [-1, 1]".into(),
        ));
    }
    Ok(LpcValue::Float(x.asin()))
}

/// acos(float x) -> float
///
/// Error if |x| > 1.
pub fn kf_acos(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let x = require_float(&args[0], 0)?;
    if x < -1.0 || x > 1.0 {
        return Err(LpcError::ValueError(
            "math domain error: acos argument out of range [-1, 1]".into(),
        ));
    }
    Ok(LpcValue::Float(x.acos()))
}

/// atan(float x) -> float
pub fn kf_atan(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    float_unary(ctx, args, f64::atan)
}

/// sinh(float x) -> float
pub fn kf_sinh(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    float_unary(ctx, args, f64::sinh)
}

/// cosh(float x) -> float
pub fn kf_cosh(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    float_unary(ctx, args, f64::cosh)
}

/// tanh(float x) -> float
pub fn kf_tanh(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    float_unary(ctx, args, f64::tanh)
}

/// pow(float x, float y) -> float
///
/// x raised to the power y. Error on domain issues.
pub fn kf_pow(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let x = require_float(&args[0], 0)?;
    let y = require_float(&args[1], 1)?;
    let result = x.powf(y);
    if result.is_nan() || result.is_infinite() {
        return Err(LpcError::ValueError("math domain error".into()));
    }
    Ok(LpcValue::Float(result))
}

/// fmod(float x, float y) -> float
///
/// Float modulo. Error if y is zero.
pub fn kf_fmod(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let x = require_float(&args[0], 0)?;
    let y = require_float(&args[1], 1)?;
    if y == 0.0 {
        return Err(LpcError::ValueError(
            "math domain error: fmod division by zero".into(),
        ));
    }
    Ok(LpcValue::Float(x % y))
}

/// atan2(float y, float x) -> float
///
/// Two-argument arctangent.
pub fn kf_atan2(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let y = require_float(&args[0], 0)?;
    let x = require_float(&args[1], 1)?;
    Ok(LpcValue::Float(y.atan2(x)))
}

/// ldexp(float x, int n) -> float
///
/// x * 2^n.
pub fn kf_ldexp(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let x = require_float(&args[0], 0)?;
    let n = require_int(&args[1], 1)?;
    let result = x * 2.0_f64.powi(n as i32);
    if result.is_nan() || result.is_infinite() {
        return Err(LpcError::ValueError("math domain error".into()));
    }
    Ok(LpcValue::Float(result))
}

/// frexp(float x) -> mixed*
///
/// Split float into mantissa and exponent. Returns ({mantissa, exponent})
/// where x = mantissa * 2^exponent and 0.5 <= |mantissa| < 1.0.
pub fn kf_frexp(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let x = require_float(&args[0], 0)?;
    if x == 0.0 {
        return Ok(LpcValue::Array(vec![
            LpcValue::Float(0.0),
            LpcValue::Int(0),
        ]));
    }
    // Manual frexp implementation
    let bits = x.to_bits();
    let sign = if (bits >> 63) != 0 { -1.0_f64 } else { 1.0_f64 };
    let exponent_biased = ((bits >> 52) & 0x7FF) as i64;
    let mantissa_bits = bits & 0x000F_FFFF_FFFF_FFFF;

    if exponent_biased == 0 {
        // Denormalized number
        if mantissa_bits == 0 {
            return Ok(LpcValue::Array(vec![
                LpcValue::Float(0.0),
                LpcValue::Int(0),
            ]));
        }
        // Normalize by multiplying by 2^52 and adjusting exponent
        let normalized = f64::from_bits((1023u64 << 52) | mantissa_bits);
        let val = (normalized - 1.0) * sign;
        // Recursively call frexp on normalized value
        let exp = -52 - 1022;
        return Ok(LpcValue::Array(vec![
            LpcValue::Float(val * 0.5_f64.powi(0)),
            LpcValue::Int(exp),
        ]));
    }

    let exponent = exponent_biased - 1022; // -1023 + 1 to get mantissa in [0.5, 1.0)
    let mantissa =
        f64::from_bits((0x3FE0_0000_0000_0000u64) | mantissa_bits) * sign;

    Ok(LpcValue::Array(vec![
        LpcValue::Float(mantissa),
        LpcValue::Int(exponent),
    ]))
}

/// modf(float x) -> mixed*
///
/// Split float into integer and fractional parts.
/// Returns ({integer_part, fractional_part}).
pub fn kf_modf(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let x = require_float(&args[0], 0)?;
    let int_part = x.trunc();
    let frac_part = x.fract();
    Ok(LpcValue::Array(vec![
        LpcValue::Float(int_part),
        LpcValue::Float(frac_part),
    ]))
}

/// random(int range) -> int
///
/// Generate random number.
/// - range > 0: returns 0..range-1
/// - range == 0: returns large random non-negative int
/// - range < 0: error
pub fn kf_random(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let range = require_int(&args[0], 0)?;
    if range < 0 {
        return Err(LpcError::ValueError("negative random range".into()));
    }
    let mut rng = rand::rng();
    if range == 0 {
        // DGD: return full 63-bit random value
        let val: i64 = rng.random::<i64>().abs();
        return Ok(LpcValue::Int(val));
    }
    // Return 0..range-1
    let val = (rng.random::<u64>() % range as u64) as i64;
    Ok(LpcValue::Int(val))
}
