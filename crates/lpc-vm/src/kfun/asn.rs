//! ASN (Arbitrary-Size Number) kfuns.
//!
//! Numbers are represented as big-endian two's complement byte strings stored
//! in LPC `String` values. The high bit of the first byte is the sign bit.
//! An empty string or "\0" represents zero.
//!
//! All 13 DGD ASN kfuns: add, sub, mult, div, mod, pow, modinv, lshift,
//! rshift, and, or, xor, cmp.

use crate::bytecode::LpcValue;
use super::{KfunContext, LpcError, require_int, require_string};

// ---------------------------------------------------------------------------
// Internal big-integer representation: sign + magnitude (big-endian, no
// leading zero bytes in the magnitude).
// ---------------------------------------------------------------------------

/// Decode a two's complement big-endian byte string into (negative, magnitude).
/// Magnitude is a big-endian byte vector with no leading zeros.
fn bytes_to_bigint(s: &[u8]) -> (bool, Vec<u8>) {
    if s.is_empty() {
        return (false, vec![]);
    }

    let negative = s[0] & 0x80 != 0;

    if negative {
        // Two's complement: negate to get positive magnitude.
        // Negate = bitwise NOT + 1.
        let mut mag = s.to_vec();
        // Bitwise NOT
        for b in mag.iter_mut() {
            *b = !*b;
        }
        // Add 1
        let mut carry = 1u16;
        for b in mag.iter_mut().rev() {
            let sum = *b as u16 + carry;
            *b = sum as u8;
            carry = sum >> 8;
        }
        strip_leading_zeros(&mut mag);
        (true, mag)
    } else {
        let mut mag = s.to_vec();
        strip_leading_zeros(&mut mag);
        (false, mag)
    }
}

/// Encode (negative, magnitude) back to a two's complement big-endian byte
/// string, returned as a Rust `String` where each byte maps to the
/// corresponding Latin-1 character.
fn bigint_to_bytes(negative: bool, magnitude: &[u8]) -> String {
    if is_zero(magnitude) {
        return String::from("\0");
    }

    if !negative {
        // Positive: just prefix a 0x00 byte if the high bit is set.
        let mut out = magnitude.to_vec();
        if out[0] & 0x80 != 0 {
            out.insert(0, 0x00);
        }
        bytes_to_lpc_string(&out)
    } else {
        // Negative: negate magnitude to get two's complement.
        // First ensure magnitude has room for sign bit interpretation.
        let mut val = magnitude.to_vec();
        // If high bit is set, add a leading zero so that after negation the
        // sign bit is correctly 1.
        if val[0] & 0x80 != 0 {
            val.insert(0, 0x00);
        }

        // Subtract 1 then bitwise NOT (equivalent to two's complement negation).
        // Subtract 1:
        let mut borrow = 1u16;
        for b in val.iter_mut().rev() {
            let sub = (*b as u16).wrapping_sub(borrow);
            *b = sub as u8;
            borrow = if sub > 0xFF { 1 } else { 0 };
        }
        // Bitwise NOT
        for b in val.iter_mut() {
            *b = !*b;
        }

        // Strip unnecessary leading 0xFF bytes (but keep at least one byte,
        // and keep enough so the sign bit remains 1).
        while val.len() > 1 && val[0] == 0xFF && val[1] & 0x80 != 0 {
            val.remove(0);
        }

        bytes_to_lpc_string(&val)
    }
}

/// Convert raw bytes to an LPC string (Latin-1 encoding: each byte becomes
/// the Unicode code point with the same value).
fn bytes_to_lpc_string(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

/// Extract the raw bytes from an LPC string argument.
fn lpc_string_to_bytes(s: &str) -> Vec<u8> {
    // Each char in a DGD LPC string is a byte value 0-255.
    // Rust strings store this as UTF-8, so we convert back.
    s.chars().map(|c| c as u8).collect()
}

fn strip_leading_zeros(v: &mut Vec<u8>) {
    while v.len() > 1 && v[0] == 0 {
        v.remove(0);
    }
    if v.is_empty() || (v.len() == 1 && v[0] == 0) {
        v.clear();
    }
}

fn is_zero(mag: &[u8]) -> bool {
    mag.is_empty() || mag.iter().all(|&b| b == 0)
}

// ---------------------------------------------------------------------------
// Unsigned magnitude arithmetic (big-endian byte vectors, no leading zeros).
// ---------------------------------------------------------------------------

/// Compare two unsigned magnitudes. Returns -1, 0, or 1.
fn mag_cmp(a: &[u8], b: &[u8]) -> i8 {
    if a.len() != b.len() {
        return if a.len() > b.len() { 1 } else { -1 };
    }
    for (x, y) in a.iter().zip(b.iter()) {
        if x != y {
            return if x > y { 1 } else { -1 };
        }
    }
    0
}

/// Add two unsigned magnitudes.
fn mag_add(a: &[u8], b: &[u8]) -> Vec<u8> {
    let max_len = a.len().max(b.len());
    let mut result = vec![0u8; max_len + 1];
    let mut carry = 0u16;

    for i in 0..max_len {
        let ai = if i < a.len() {
            a[a.len() - 1 - i] as u16
        } else {
            0
        };
        let bi = if i < b.len() {
            b[b.len() - 1 - i] as u16
        } else {
            0
        };
        let sum = ai + bi + carry;
        result[max_len - i] = sum as u8;
        carry = sum >> 8;
    }
    result[0] = carry as u8;

    let mut r = result;
    strip_leading_zeros(&mut r);
    r
}

/// Subtract two unsigned magnitudes: a - b. Assumes a >= b.
fn mag_sub(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut result = a.to_vec();
    let mut borrow = 0i16;

    for i in 0..a.len() {
        let ai = a[a.len() - 1 - i] as i16;
        let bi = if i < b.len() {
            b[b.len() - 1 - i] as i16
        } else {
            0
        };
        let diff = ai - bi - borrow;
        if diff < 0 {
            result[a.len() - 1 - i] = (diff + 256) as u8;
            borrow = 1;
        } else {
            result[a.len() - 1 - i] = diff as u8;
            borrow = 0;
        }
    }

    strip_leading_zeros(&mut result);
    result
}

/// Multiply two unsigned magnitudes.
fn mag_mul(a: &[u8], b: &[u8]) -> Vec<u8> {
    if is_zero(a) || is_zero(b) {
        return vec![];
    }

    let mut result = vec![0u8; a.len() + b.len()];

    for i in 0..a.len() {
        let mut carry = 0u32;
        let ai = a[a.len() - 1 - i] as u32;
        for j in 0..b.len() {
            let bj = b[b.len() - 1 - j] as u32;
            let pos = result.len() - 1 - i - j;
            let prod = ai * bj + result[pos] as u32 + carry;
            result[pos] = prod as u8;
            carry = prod >> 8;
        }
        let pos = result.len() - 1 - i - b.len();
        result[pos] = (result[pos] as u32 + carry) as u8;
    }

    strip_leading_zeros(&mut result);
    result
}

/// Divide unsigned magnitude `a` by `b`, returning (quotient, remainder).
/// Panics if `b` is zero (caller must check).
fn mag_divmod(a: &[u8], b: &[u8]) -> (Vec<u8>, Vec<u8>) {
    debug_assert!(!is_zero(b), "division by zero");

    if is_zero(a) {
        return (vec![], vec![]);
    }

    if mag_cmp(a, b) < 0 {
        return (vec![], a.to_vec());
    }

    if b.len() == 1 {
        // Fast path: single-byte divisor.
        return mag_divmod_short(a, b[0]);
    }

    // Long division in base 256, using Knuth Algorithm D simplified.
    mag_divmod_long(a, b)
}

/// Division by a single byte.
fn mag_divmod_short(a: &[u8], b: u8) -> (Vec<u8>, Vec<u8>) {
    let divisor = b as u16;
    let mut quotient = Vec::with_capacity(a.len());
    let mut rem = 0u16;

    for &byte in a {
        let cur = (rem << 8) | byte as u16;
        quotient.push((cur / divisor) as u8);
        rem = cur % divisor;
    }

    strip_leading_zeros(&mut quotient);
    let remainder = if rem == 0 {
        vec![]
    } else {
        vec![rem as u8]
    };
    (quotient, remainder)
}

/// Long division for multi-byte divisors.
fn mag_divmod_long(a: &[u8], b: &[u8]) -> (Vec<u8>, Vec<u8>) {
    // We implement division using repeated subtraction with estimation,
    // working at the bit level for correctness.
    let a_bits = mag_bit_length(a);
    let b_bits = mag_bit_length(b);

    if a_bits < b_bits {
        return (vec![], a.to_vec());
    }

    let mut quotient_bits = vec![false; a_bits - b_bits + 1];
    let mut remainder = a.to_vec();

    for i in (0..=(a_bits - b_bits)).rev() {
        // Compare remainder with b << i
        let shifted = mag_shl_bits(b, i);
        if mag_cmp(&remainder, &shifted) >= 0 {
            remainder = mag_sub(&remainder, &shifted);
            quotient_bits[a_bits - b_bits - i] = true;
        }
    }

    let quotient = bits_to_mag(&quotient_bits);
    (quotient, remainder)
}

/// Convert a vector of bits (MSB first) to magnitude bytes.
fn bits_to_mag(bits: &[bool]) -> Vec<u8> {
    if bits.is_empty() || bits.iter().all(|&b| !b) {
        return vec![];
    }

    // Find first set bit
    let first_set = bits.iter().position(|&b| b).unwrap();
    let significant = &bits[first_set..];

    let byte_count = (significant.len() + 7) / 8;
    let mut result = vec![0u8; byte_count];

    for (i, &bit) in significant.iter().rev().enumerate() {
        if bit {
            result[byte_count - 1 - i / 8] |= 1 << (i % 8);
        }
    }

    strip_leading_zeros(&mut result);
    result
}

/// Number of bits in magnitude (0 for zero).
fn mag_bit_length(a: &[u8]) -> usize {
    if is_zero(a) {
        return 0;
    }
    let leading = a[0].leading_zeros() as usize;
    a.len() * 8 - leading
}

/// Left-shift magnitude by `n` bits.
fn mag_shl_bits(a: &[u8], n: usize) -> Vec<u8> {
    if is_zero(a) || n == 0 {
        return a.to_vec();
    }

    let byte_shift = n / 8;
    let bit_shift = n % 8;

    // Apply the sub-byte bit shift first, producing an intermediate result
    // that may be one byte longer than the input.
    let mut shifted = vec![0u8; a.len() + 1];
    if bit_shift == 0 {
        shifted[1..].copy_from_slice(a);
    } else {
        for i in 0..a.len() {
            let val = (a[i] as u16) << bit_shift;
            shifted[i] |= (val >> 8) as u8;
            shifted[i + 1] |= val as u8;
        }
    }

    // Append `byte_shift` zero bytes (equivalent to multiplying by 256^byte_shift).
    shifted.resize(shifted.len() + byte_shift, 0);

    strip_leading_zeros(&mut shifted);
    shifted
}

/// Right-shift magnitude by `n` bits.
fn mag_shr_bits(a: &[u8], n: usize) -> Vec<u8> {
    if is_zero(a) || n == 0 {
        return a.to_vec();
    }

    let byte_shift = n / 8;
    let bit_shift = n % 8;

    if byte_shift >= a.len() {
        return vec![];
    }

    let effective = &a[..a.len() - byte_shift];
    if effective.is_empty() {
        return vec![];
    }

    let mut result = vec![0u8; effective.len()];

    for i in (0..effective.len()).rev() {
        result[i] = effective[i] >> bit_shift;
        if i > 0 {
            result[i] |= effective[i - 1] << (8 - bit_shift);
        }
    }

    strip_leading_zeros(&mut result);
    result
}

// ---------------------------------------------------------------------------
// Signed big-integer arithmetic (using sign + magnitude representation).
// ---------------------------------------------------------------------------

/// Add two signed numbers.
fn signed_add(
    a_neg: bool,
    a_mag: &[u8],
    b_neg: bool,
    b_mag: &[u8],
) -> (bool, Vec<u8>) {
    if a_neg == b_neg {
        // Same sign: add magnitudes, keep sign.
        (a_neg, mag_add(a_mag, b_mag))
    } else {
        // Different signs: subtract smaller from larger.
        match mag_cmp(a_mag, b_mag) {
            1 | 0 => {
                let diff = mag_sub(a_mag, b_mag);
                if is_zero(&diff) {
                    (false, vec![])
                } else {
                    (a_neg, diff)
                }
            }
            _ => {
                let diff = mag_sub(b_mag, a_mag);
                if is_zero(&diff) {
                    (false, vec![])
                } else {
                    (b_neg, diff)
                }
            }
        }
    }
}

/// Subtract: a - b = a + (-b).
fn signed_sub(
    a_neg: bool,
    a_mag: &[u8],
    b_neg: bool,
    b_mag: &[u8],
) -> (bool, Vec<u8>) {
    signed_add(a_neg, a_mag, !b_neg, b_mag)
}

/// Multiply two signed numbers.
fn signed_mul(
    a_neg: bool,
    a_mag: &[u8],
    b_neg: bool,
    b_mag: &[u8],
) -> (bool, Vec<u8>) {
    let mag = mag_mul(a_mag, b_mag);
    if is_zero(&mag) {
        (false, vec![])
    } else {
        (a_neg ^ b_neg, mag)
    }
}

/// Divide two signed numbers (truncating toward zero), returning (quotient, remainder).
/// The remainder has the same sign as the dividend.
fn signed_divmod(
    a_neg: bool,
    a_mag: &[u8],
    b_neg: bool,
    b_mag: &[u8],
) -> (bool, Vec<u8>, bool, Vec<u8>) {
    let (q, r) = mag_divmod(a_mag, b_mag);
    let q_neg = if is_zero(&q) { false } else { a_neg ^ b_neg };
    let r_neg = if is_zero(&r) { false } else { a_neg };
    (q_neg, q, r_neg, r)
}

/// Euclidean modulo: result has the same sign convention as the modulus in DGD.
/// DGD's ASN mod returns a non-negative result when the modulus is positive.
fn signed_mod(
    a_neg: bool,
    a_mag: &[u8],
    b_neg: bool,
    b_mag: &[u8],
) -> (bool, Vec<u8>) {
    let (_q_neg, _q, r_neg, r) = signed_divmod(a_neg, a_mag, b_neg, b_mag);
    if is_zero(&r) {
        return (false, vec![]);
    }
    // If remainder is negative and modulus is positive (or vice versa),
    // adjust: r = r + |b|
    if r_neg != b_neg {
        let adjusted = mag_sub(b_mag, &r);
        (b_neg, adjusted)
    } else {
        (r_neg, r)
    }
}

/// Modular exponentiation: base^exp mod modulus.
/// Uses square-and-multiply algorithm.
fn mod_pow(
    base_neg: bool,
    base_mag: &[u8],
    exp_neg: bool,
    exp_mag: &[u8],
    mod_neg: bool,
    mod_mag: &[u8],
) -> Result<(bool, Vec<u8>), LpcError> {
    if is_zero(mod_mag) {
        return Err(LpcError::ValueError("asn_pow: zero modulus".into()));
    }

    if exp_neg {
        return Err(LpcError::ValueError(
            "asn_pow: negative exponent".into(),
        ));
    }

    if is_zero(exp_mag) {
        // x^0 = 1 mod m (if m > 1, result is 1; if m == 1, result is 0)
        let one = vec![1u8];
        if mag_cmp(mod_mag, &one) <= 0 {
            return Ok((false, vec![]));
        }
        return Ok((false, one));
    }

    // Reduce base mod modulus first.
    let (b_neg, b_mag) = reduce_mod(base_neg, base_mag, mod_neg, mod_mag);

    // Square-and-multiply
    let bits = mag_bit_length(exp_mag);
    let mut result_neg = false;
    let mut result_mag: Vec<u8> = vec![1u8];

    for i in 0..bits {
        let bit_idx = bits - 1 - i;
        let byte_idx = exp_mag.len() - 1 - bit_idx / 8;
        let bit = (exp_mag[byte_idx] >> (bit_idx % 8)) & 1;

        // Square
        let (sq_neg, sq_mag) = signed_mul(result_neg, &result_mag, result_neg, &result_mag);
        let (rn, rm) = reduce_mod(sq_neg, &sq_mag, mod_neg, mod_mag);
        result_neg = rn;
        result_mag = rm;

        if bit == 1 {
            // Multiply by base
            let (mn, mm) = signed_mul(result_neg, &result_mag, b_neg, &b_mag);
            let (rn2, rm2) = reduce_mod(mn, &mm, mod_neg, mod_mag);
            result_neg = rn2;
            result_mag = rm2;
        }
    }

    Ok((result_neg, result_mag))
}

/// Reduce a signed value modulo m, giving a non-negative result when m > 0.
fn reduce_mod(
    a_neg: bool,
    a_mag: &[u8],
    _m_neg: bool,
    m_mag: &[u8],
) -> (bool, Vec<u8>) {
    if is_zero(a_mag) {
        return (false, vec![]);
    }
    let (_q, r) = mag_divmod(a_mag, m_mag);
    if is_zero(&r) {
        return (false, vec![]);
    }
    if a_neg {
        // Negative value: result = |m| - r
        let adjusted = mag_sub(m_mag, &r);
        (false, adjusted)
    } else {
        (false, r)
    }
}

/// Extended Euclidean algorithm: given a and m, find x such that a*x = 1 (mod m).
/// Returns an error if gcd(a, m) != 1.
fn mod_inverse(
    a_neg: bool,
    a_mag: &[u8],
    m_neg: bool,
    m_mag: &[u8],
) -> Result<(bool, Vec<u8>), LpcError> {
    if is_zero(m_mag) {
        return Err(LpcError::ValueError(
            "asn_modinv: zero modulus".into(),
        ));
    }
    if is_zero(a_mag) {
        return Err(LpcError::ValueError(
            "asn_modinv: zero has no modular inverse".into(),
        ));
    }

    // Work with reduced a.
    let (_, a_reduced) = reduce_mod(a_neg, a_mag, m_neg, m_mag);
    if is_zero(&a_reduced) {
        return Err(LpcError::ValueError(
            "asn_modinv: value is divisible by modulus".into(),
        ));
    }

    // Extended Euclidean algorithm.
    // We track: old_r, r, old_s, s where gcd = old_r, inverse = old_s.
    let mut old_r = m_mag.to_vec();
    let mut r = a_reduced;
    let mut old_s_neg = false;
    let mut old_s: Vec<u8> = vec![];
    let mut s_neg = false;
    let mut s: Vec<u8> = vec![1u8];

    while !is_zero(&r) {
        let (q, remainder) = mag_divmod(&old_r, &r);

        // Update r: old_r = r, r = remainder
        old_r = r;
        r = remainder;

        // Update s: old_s, s = s, old_s - q * s
        let (qs_neg, qs_mag) = signed_mul(false, &q, s_neg, &s);
        let (new_s_neg, new_s_mag) = signed_sub(old_s_neg, &old_s, qs_neg, &qs_mag);
        old_s_neg = s_neg;
        old_s = s;
        s_neg = new_s_neg;
        s = new_s_mag;
    }

    // gcd = old_r, must be 1
    if old_r.len() != 1 || old_r[0] != 1 {
        return Err(LpcError::ValueError(
            "asn_modinv: value and modulus are not coprime".into(),
        ));
    }

    // Reduce the inverse modulo m to ensure positive result
    let (res_neg, res_mag) = reduce_mod(old_s_neg, &old_s, m_neg, m_mag);
    Ok((res_neg, res_mag))
}

// ---------------------------------------------------------------------------
// Bitwise operations on two's complement byte representations.
// ---------------------------------------------------------------------------

/// Perform a bitwise operation on two's complement byte strings.
/// The operation is applied byte-by-byte after sign-extending to equal length.
fn bitwise_op(a: &[u8], b: &[u8], op: fn(u8, u8) -> u8) -> Vec<u8> {
    let a_sign = if a.is_empty() { 0x00 } else { if a[0] & 0x80 != 0 { 0xFF } else { 0x00 } };
    let b_sign = if b.is_empty() { 0x00 } else { if b[0] & 0x80 != 0 { 0xFF } else { 0x00 } };

    // Use the raw two's complement representations, not magnitude.
    let a_bytes = if a.is_empty() { &[0u8][..] } else { a };
    let b_bytes = if b.is_empty() { &[0u8][..] } else { b };

    let max_len = a_bytes.len().max(b_bytes.len());
    let mut result = vec![0u8; max_len];

    for i in 0..max_len {
        let ai = if i < max_len - a_bytes.len() {
            a_sign
        } else {
            a_bytes[i - (max_len - a_bytes.len())]
        };
        let bi = if i < max_len - b_bytes.len() {
            b_sign
        } else {
            b_bytes[i - (max_len - b_bytes.len())]
        };
        result[i] = op(ai, bi);
    }

    // Determine result sign extension byte
    let result_sign = if !result.is_empty() && result[0] & 0x80 != 0 {
        0xFF
    } else {
        0x00
    };

    // Strip redundant leading sign bytes
    while result.len() > 1 && result[0] == result_sign && (result[1] & 0x80 != 0) == (result_sign == 0xFF) {
        result.remove(0);
    }

    result
}

// ---------------------------------------------------------------------------
// Tick cost estimation.
// ---------------------------------------------------------------------------

/// Charge ticks proportional to the byte sizes of the operands.
fn charge_ticks(ctx: &mut KfunContext, sizes: &[usize]) {
    let total: usize = sizes.iter().sum();
    // Base cost of 10 plus 1 tick per byte of operand data.
    let cost = 10u64 + total as u64;
    *ctx.tick_counter = ctx.tick_counter.saturating_sub(cost);
}

/// Charge ticks for expensive operations (multiplication, exponentiation, etc.).
fn charge_ticks_expensive(ctx: &mut KfunContext, sizes: &[usize]) {
    let total: usize = sizes.iter().sum();
    // Quadratic cost for multiply/divide/pow-style operations.
    let cost = 10u64 + (total as u64) * (total as u64 / 256 + 1);
    *ctx.tick_counter = ctx.tick_counter.saturating_sub(cost);
}

// ---------------------------------------------------------------------------
// Public kfun implementations.
// ---------------------------------------------------------------------------

/// asn_add(string s1, string s2, string modulus) -> string
///
/// Returns (s1 + s2) % modulus.
pub fn kf_asn_add(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s1 = require_string(&args[0], 0)?;
    let s2 = require_string(&args[1], 1)?;
    let s3 = require_string(&args[2], 2)?;

    let b1 = lpc_string_to_bytes(s1);
    let b2 = lpc_string_to_bytes(s2);
    let b3 = lpc_string_to_bytes(s3);

    charge_ticks(ctx, &[b1.len(), b2.len(), b3.len()]);

    let (a_neg, a_mag) = bytes_to_bigint(&b1);
    let (b_neg, b_mag) = bytes_to_bigint(&b2);
    let (m_neg, m_mag) = bytes_to_bigint(&b3);

    if is_zero(&m_mag) {
        return Err(LpcError::ValueError("asn_add: zero modulus".into()));
    }

    let (r_neg, r_mag) = signed_add(a_neg, &a_mag, b_neg, &b_mag);
    let (res_neg, res_mag) = reduce_mod(r_neg, &r_mag, m_neg, &m_mag);

    Ok(LpcValue::String(bigint_to_bytes(res_neg, &res_mag)))
}

/// asn_sub(string s1, string s2, string modulus) -> string
///
/// Returns (s1 - s2) % modulus.
pub fn kf_asn_sub(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s1 = require_string(&args[0], 0)?;
    let s2 = require_string(&args[1], 1)?;
    let s3 = require_string(&args[2], 2)?;

    let b1 = lpc_string_to_bytes(s1);
    let b2 = lpc_string_to_bytes(s2);
    let b3 = lpc_string_to_bytes(s3);

    charge_ticks(ctx, &[b1.len(), b2.len(), b3.len()]);

    let (a_neg, a_mag) = bytes_to_bigint(&b1);
    let (b_neg, b_mag) = bytes_to_bigint(&b2);
    let (m_neg, m_mag) = bytes_to_bigint(&b3);

    if is_zero(&m_mag) {
        return Err(LpcError::ValueError("asn_sub: zero modulus".into()));
    }

    let (r_neg, r_mag) = signed_sub(a_neg, &a_mag, b_neg, &b_mag);
    let (res_neg, res_mag) = reduce_mod(r_neg, &r_mag, m_neg, &m_mag);

    Ok(LpcValue::String(bigint_to_bytes(res_neg, &res_mag)))
}

/// asn_mult(string s1, string s2, string modulus) -> string
///
/// Returns (s1 * s2) % modulus.
pub fn kf_asn_mult(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s1 = require_string(&args[0], 0)?;
    let s2 = require_string(&args[1], 1)?;
    let s3 = require_string(&args[2], 2)?;

    let b1 = lpc_string_to_bytes(s1);
    let b2 = lpc_string_to_bytes(s2);
    let b3 = lpc_string_to_bytes(s3);

    charge_ticks_expensive(ctx, &[b1.len(), b2.len(), b3.len()]);

    let (a_neg, a_mag) = bytes_to_bigint(&b1);
    let (b_neg, b_mag) = bytes_to_bigint(&b2);
    let (m_neg, m_mag) = bytes_to_bigint(&b3);

    if is_zero(&m_mag) {
        return Err(LpcError::ValueError("asn_mult: zero modulus".into()));
    }

    let (r_neg, r_mag) = signed_mul(a_neg, &a_mag, b_neg, &b_mag);
    let (res_neg, res_mag) = reduce_mod(r_neg, &r_mag, m_neg, &m_mag);

    Ok(LpcValue::String(bigint_to_bytes(res_neg, &res_mag)))
}

/// asn_div(string s1, string s2, string modulus) -> string
///
/// Returns (s1 / s2) % modulus (truncating division).
pub fn kf_asn_div(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s1 = require_string(&args[0], 0)?;
    let s2 = require_string(&args[1], 1)?;
    let s3 = require_string(&args[2], 2)?;

    let b1 = lpc_string_to_bytes(s1);
    let b2 = lpc_string_to_bytes(s2);
    let b3 = lpc_string_to_bytes(s3);

    charge_ticks_expensive(ctx, &[b1.len(), b2.len(), b3.len()]);

    let (a_neg, a_mag) = bytes_to_bigint(&b1);
    let (b_neg, b_mag) = bytes_to_bigint(&b2);
    let (m_neg, m_mag) = bytes_to_bigint(&b3);

    if is_zero(&b_mag) {
        return Err(LpcError::ValueError("asn_div: division by zero".into()));
    }
    if is_zero(&m_mag) {
        return Err(LpcError::ValueError("asn_div: zero modulus".into()));
    }

    let (q_neg, q_mag, _r_neg, _r_mag) = signed_divmod(a_neg, &a_mag, b_neg, &b_mag);
    let (res_neg, res_mag) = reduce_mod(q_neg, &q_mag, m_neg, &m_mag);

    Ok(LpcValue::String(bigint_to_bytes(res_neg, &res_mag)))
}

/// asn_mod(string s1, string s2) -> string
///
/// Returns s1 % s2.
pub fn kf_asn_mod(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s1 = require_string(&args[0], 0)?;
    let s2 = require_string(&args[1], 1)?;

    let b1 = lpc_string_to_bytes(s1);
    let b2 = lpc_string_to_bytes(s2);

    charge_ticks_expensive(ctx, &[b1.len(), b2.len()]);

    let (a_neg, a_mag) = bytes_to_bigint(&b1);
    let (b_neg, b_mag) = bytes_to_bigint(&b2);

    if is_zero(&b_mag) {
        return Err(LpcError::ValueError("asn_mod: division by zero".into()));
    }

    let (r_neg, r_mag) = signed_mod(a_neg, &a_mag, b_neg, &b_mag);

    Ok(LpcValue::String(bigint_to_bytes(r_neg, &r_mag)))
}

/// asn_pow(string base, string exponent, string modulus) -> string
///
/// Modular exponentiation: (base ** exponent) % modulus.
pub fn kf_asn_pow(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s1 = require_string(&args[0], 0)?;
    let s2 = require_string(&args[1], 1)?;
    let s3 = require_string(&args[2], 2)?;

    let b1 = lpc_string_to_bytes(s1);
    let b2 = lpc_string_to_bytes(s2);
    let b3 = lpc_string_to_bytes(s3);

    charge_ticks_expensive(ctx, &[b1.len(), b2.len(), b3.len()]);

    let (base_neg, base_mag) = bytes_to_bigint(&b1);
    let (exp_neg, exp_mag) = bytes_to_bigint(&b2);
    let (mod_neg, mod_mag) = bytes_to_bigint(&b3);

    let (r_neg, r_mag) = mod_pow(base_neg, &base_mag, exp_neg, &exp_mag, mod_neg, &mod_mag)?;

    Ok(LpcValue::String(bigint_to_bytes(r_neg, &r_mag)))
}

/// asn_modinv(string s1, string s2) -> string
///
/// Modular inverse: s1^-1 mod s2.
pub fn kf_asn_modinv(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s1 = require_string(&args[0], 0)?;
    let s2 = require_string(&args[1], 1)?;

    let b1 = lpc_string_to_bytes(s1);
    let b2 = lpc_string_to_bytes(s2);

    charge_ticks_expensive(ctx, &[b1.len(), b2.len()]);

    let (a_neg, a_mag) = bytes_to_bigint(&b1);
    let (m_neg, m_mag) = bytes_to_bigint(&b2);

    let (r_neg, r_mag) = mod_inverse(a_neg, &a_mag, m_neg, &m_mag)?;

    Ok(LpcValue::String(bigint_to_bytes(r_neg, &r_mag)))
}

/// asn_lshift(string s1, int nbits, string modulus) -> string
///
/// Left shift s1 by nbits bits, then reduce mod modulus.
pub fn kf_asn_lshift(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s1 = require_string(&args[0], 0)?;
    let nbits = require_int(&args[1], 1)?;
    let s3 = require_string(&args[2], 2)?;

    let b1 = lpc_string_to_bytes(s1);
    let b3 = lpc_string_to_bytes(s3);

    charge_ticks(ctx, &[b1.len(), b3.len(), nbits.unsigned_abs() as usize / 8]);

    if nbits < 0 {
        return Err(LpcError::ValueError(
            "asn_lshift: negative shift count".into(),
        ));
    }

    let (a_neg, a_mag) = bytes_to_bigint(&b1);
    let (m_neg, m_mag) = bytes_to_bigint(&b3);

    if is_zero(&m_mag) {
        return Err(LpcError::ValueError("asn_lshift: zero modulus".into()));
    }

    let shifted = mag_shl_bits(&a_mag, nbits as usize);
    let (res_neg, res_mag) = reduce_mod(a_neg, &shifted, m_neg, &m_mag);

    Ok(LpcValue::String(bigint_to_bytes(res_neg, &res_mag)))
}

/// asn_rshift(string s1, int nbits) -> string
///
/// Arithmetic right shift s1 by nbits bits.
pub fn kf_asn_rshift(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s1 = require_string(&args[0], 0)?;
    let nbits = require_int(&args[1], 1)?;

    let b1 = lpc_string_to_bytes(s1);

    charge_ticks(ctx, &[b1.len(), nbits.unsigned_abs() as usize / 8]);

    if nbits < 0 {
        return Err(LpcError::ValueError(
            "asn_rshift: negative shift count".into(),
        ));
    }

    let (a_neg, a_mag) = bytes_to_bigint(&b1);

    if is_zero(&a_mag) {
        return Ok(LpcValue::String(String::from("\0")));
    }

    let shifted = mag_shr_bits(&a_mag, nbits as usize);

    if is_zero(&shifted) {
        // For negative numbers, arithmetic right shift toward -1, not 0.
        if a_neg {
            // -1 in two's complement is 0xFF
            return Ok(LpcValue::String(bigint_to_bytes(true, &[1u8])));
        }
        return Ok(LpcValue::String(String::from("\0")));
    }

    Ok(LpcValue::String(bigint_to_bytes(a_neg, &shifted)))
}

/// asn_and(string s1, string s2) -> string
///
/// Bitwise AND of two ASN values.
pub fn kf_asn_and(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s1 = require_string(&args[0], 0)?;
    let s2 = require_string(&args[1], 1)?;

    let b1 = lpc_string_to_bytes(s1);
    let b2 = lpc_string_to_bytes(s2);

    charge_ticks(ctx, &[b1.len(), b2.len()]);

    let tc1 = if b1.is_empty() { vec![0u8] } else { b1 };
    let tc2 = if b2.is_empty() { vec![0u8] } else { b2 };

    let result = bitwise_op(&tc1, &tc2, |a, b| a & b);

    if result.iter().all(|&b| b == 0) {
        return Ok(LpcValue::String(String::from("\0")));
    }

    Ok(LpcValue::String(bytes_to_lpc_string(&result)))
}

/// asn_or(string s1, string s2) -> string
///
/// Bitwise OR of two ASN values.
pub fn kf_asn_or(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s1 = require_string(&args[0], 0)?;
    let s2 = require_string(&args[1], 1)?;

    let b1 = lpc_string_to_bytes(s1);
    let b2 = lpc_string_to_bytes(s2);

    charge_ticks(ctx, &[b1.len(), b2.len()]);

    let tc1 = if b1.is_empty() { vec![0u8] } else { b1 };
    let tc2 = if b2.is_empty() { vec![0u8] } else { b2 };

    let result = bitwise_op(&tc1, &tc2, |a, b| a | b);

    if result.iter().all(|&b| b == 0) {
        return Ok(LpcValue::String(String::from("\0")));
    }

    Ok(LpcValue::String(bytes_to_lpc_string(&result)))
}

/// asn_xor(string s1, string s2) -> string
///
/// Bitwise XOR of two ASN values.
pub fn kf_asn_xor(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s1 = require_string(&args[0], 0)?;
    let s2 = require_string(&args[1], 1)?;

    let b1 = lpc_string_to_bytes(s1);
    let b2 = lpc_string_to_bytes(s2);

    charge_ticks(ctx, &[b1.len(), b2.len()]);

    let tc1 = if b1.is_empty() { vec![0u8] } else { b1 };
    let tc2 = if b2.is_empty() { vec![0u8] } else { b2 };

    let result = bitwise_op(&tc1, &tc2, |a, b| a ^ b);

    if result.iter().all(|&b| b == 0) {
        return Ok(LpcValue::String(String::from("\0")));
    }

    Ok(LpcValue::String(bytes_to_lpc_string(&result)))
}

/// asn_cmp(string s1, string s2) -> int
///
/// Compare two ASN values. Returns -1, 0, or 1.
pub fn kf_asn_cmp(ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let s1 = require_string(&args[0], 0)?;
    let s2 = require_string(&args[1], 1)?;

    let b1 = lpc_string_to_bytes(s1);
    let b2 = lpc_string_to_bytes(s2);

    charge_ticks(ctx, &[b1.len(), b2.len()]);

    let (a_neg, a_mag) = bytes_to_bigint(&b1);
    let (b_neg, b_mag) = bytes_to_bigint(&b2);

    let result = if a_neg && !b_neg {
        if is_zero(&a_mag) && is_zero(&b_mag) {
            0i64
        } else {
            -1i64
        }
    } else if !a_neg && b_neg {
        if is_zero(&a_mag) && is_zero(&b_mag) {
            0i64
        } else {
            1i64
        }
    } else if a_neg {
        // Both negative: larger magnitude means smaller value.
        -(mag_cmp(&a_mag, &b_mag) as i64)
    } else {
        // Both positive.
        mag_cmp(&a_mag, &b_mag) as i64
    };

    Ok(LpcValue::Int(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::ObjectRef;

    fn make_ctx(ticks: &mut u64) -> KfunContext<'_> {
        let obj = Box::leak(Box::new(ObjectRef {
            id: 0,
            path: "test".into(),
            is_lightweight: false,
        }));
        KfunContext {
            this_object: obj,
            previous_object: None,
            tick_counter: ticks,
        }
    }

    /// Encode an i64 as a big-endian two's complement byte string.
    fn int_to_asn(val: i64) -> LpcValue {
        if val == 0 {
            return LpcValue::String(String::from("\0"));
        }

        let negative = val < 0;
        let abs_val = if val == i64::MIN {
            // Handle overflow for i64::MIN
            (i64::MAX as u64) + 1
        } else {
            val.unsigned_abs()
        };

        // Convert to big-endian bytes.
        let be = abs_val.to_be_bytes();
        // Find first non-zero byte.
        let start = be.iter().position(|&b| b != 0).unwrap_or(be.len() - 1);
        let mag: Vec<u8> = be[start..].to_vec();

        LpcValue::String(bigint_to_bytes(negative, &mag))
    }

    /// Decode an ASN LPC string back to i64 (for small values).
    fn asn_to_int(val: &LpcValue) -> i64 {
        let s = val.as_string().unwrap();
        let bytes = lpc_string_to_bytes(s);
        let (neg, mag) = bytes_to_bigint(&bytes);
        if is_zero(&mag) {
            return 0;
        }
        let mut result: i64 = 0;
        for &b in &mag {
            result = (result << 8) | b as i64;
        }
        if neg {
            -result
        } else {
            result
        }
    }

    #[test]
    fn test_roundtrip_positive() {
        for val in [0i64, 1, 127, 128, 255, 256, 1000, 65535, 0x7FFF_FFFF] {
            let encoded = int_to_asn(val);
            let decoded = asn_to_int(&encoded);
            assert_eq!(decoded, val, "roundtrip failed for {}", val);
        }
    }

    #[test]
    fn test_roundtrip_negative() {
        for val in [-1i64, -128, -129, -256, -1000, -65535] {
            let encoded = int_to_asn(val);
            let decoded = asn_to_int(&encoded);
            assert_eq!(decoded, val, "roundtrip failed for {}", val);
        }
    }

    #[test]
    fn test_add_basic() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 10 + 20 mod 100 = 30
        let args = [int_to_asn(10), int_to_asn(20), int_to_asn(100)];
        let result = kf_asn_add(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 30);
    }

    #[test]
    fn test_add_with_modulus() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 80 + 50 mod 100 = 30
        let args = [int_to_asn(80), int_to_asn(50), int_to_asn(100)];
        let result = kf_asn_add(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 30);
    }

    #[test]
    fn test_sub_basic() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 50 - 20 mod 100 = 30
        let args = [int_to_asn(50), int_to_asn(20), int_to_asn(100)];
        let result = kf_asn_sub(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 30);
    }

    #[test]
    fn test_sub_negative_result() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 20 - 50 mod 100 = 70  (reduced mod 100)
        let args = [int_to_asn(20), int_to_asn(50), int_to_asn(100)];
        let result = kf_asn_sub(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 70);
    }

    #[test]
    fn test_mult_basic() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 7 * 8 mod 100 = 56
        let args = [int_to_asn(7), int_to_asn(8), int_to_asn(100)];
        let result = kf_asn_mult(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 56);
    }

    #[test]
    fn test_mult_with_reduction() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 15 * 15 mod 100 = 225 mod 100 = 25
        let args = [int_to_asn(15), int_to_asn(15), int_to_asn(100)];
        let result = kf_asn_mult(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 25);
    }

    #[test]
    fn test_div_basic() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 100 / 10 mod 1000 = 10
        let args = [int_to_asn(100), int_to_asn(10), int_to_asn(1000)];
        let result = kf_asn_div(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 10);
    }

    #[test]
    fn test_div_by_zero() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        let args = [int_to_asn(100), int_to_asn(0), int_to_asn(1000)];
        assert!(kf_asn_div(&mut ctx, &args).is_err());
    }

    #[test]
    fn test_mod_basic() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 17 % 5 = 2
        let args = [int_to_asn(17), int_to_asn(5)];
        let result = kf_asn_mod(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 2);
    }

    #[test]
    fn test_mod_zero_result() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 15 % 5 = 0
        let args = [int_to_asn(15), int_to_asn(5)];
        let result = kf_asn_mod(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 0);
    }

    #[test]
    fn test_pow_basic() {
        let mut ticks = 100000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 2^10 mod 1000 = 1024 mod 1000 = 24
        let args = [int_to_asn(2), int_to_asn(10), int_to_asn(1000)];
        let result = kf_asn_pow(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 24);
    }

    #[test]
    fn test_pow_zero_exponent() {
        let mut ticks = 100000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 5^0 mod 100 = 1
        let args = [int_to_asn(5), int_to_asn(0), int_to_asn(100)];
        let result = kf_asn_pow(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 1);
    }

    #[test]
    fn test_pow_large() {
        let mut ticks = 100000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 3^7 mod 50 = 2187 mod 50 = 37
        let args = [int_to_asn(3), int_to_asn(7), int_to_asn(50)];
        let result = kf_asn_pow(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 37);
    }

    #[test]
    fn test_modinv_basic() {
        let mut ticks = 100000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 3^-1 mod 7 = 5 (because 3*5 = 15 = 2*7 + 1)
        let args = [int_to_asn(3), int_to_asn(7)];
        let result = kf_asn_modinv(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 5);
    }

    #[test]
    fn test_modinv_not_coprime() {
        let mut ticks = 100000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 4^-1 mod 8 should fail (gcd = 4)
        let args = [int_to_asn(4), int_to_asn(8)];
        assert!(kf_asn_modinv(&mut ctx, &args).is_err());
    }

    #[test]
    fn test_lshift() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 1 << 4 mod 100 = 16
        let args = [int_to_asn(1), LpcValue::Int(4), int_to_asn(100)];
        let result = kf_asn_lshift(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 16);
    }

    #[test]
    fn test_lshift_with_modulus() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 1 << 10 mod 100 = 1024 mod 100 = 24
        let args = [int_to_asn(1), LpcValue::Int(10), int_to_asn(100)];
        let result = kf_asn_lshift(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 24);
    }

    #[test]
    fn test_rshift() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 256 >> 4 = 16
        let args = [int_to_asn(256), LpcValue::Int(4)];
        let result = kf_asn_rshift(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 16);
    }

    #[test]
    fn test_rshift_to_zero() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 1 >> 8 = 0
        let args = [int_to_asn(1), LpcValue::Int(8)];
        let result = kf_asn_rshift(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 0);
    }

    #[test]
    fn test_rshift_negative() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        // -256 >> 4 = -16
        let args = [int_to_asn(-256), LpcValue::Int(4)];
        let result = kf_asn_rshift(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), -16);
    }

    #[test]
    fn test_and() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 0xFF & 0x0F = 0x0F = 15
        let args = [int_to_asn(0xFF), int_to_asn(0x0F)];
        let result = kf_asn_and(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 15);
    }

    #[test]
    fn test_or() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 0xF0 | 0x0F = 0xFF = 255
        let args = [int_to_asn(0xF0), int_to_asn(0x0F)];
        let result = kf_asn_or(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 255);
    }

    #[test]
    fn test_xor() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 0xFF ^ 0x0F = 0xF0 = 240
        let args = [int_to_asn(0xFF), int_to_asn(0x0F)];
        let result = kf_asn_xor(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 240);
    }

    #[test]
    fn test_cmp_equal() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        let args = [int_to_asn(42), int_to_asn(42)];
        let result = kf_asn_cmp(&mut ctx, &args).unwrap();
        assert_eq!(result, LpcValue::Int(0));
    }

    #[test]
    fn test_cmp_less() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        let args = [int_to_asn(10), int_to_asn(42)];
        let result = kf_asn_cmp(&mut ctx, &args).unwrap();
        assert_eq!(result, LpcValue::Int(-1));
    }

    #[test]
    fn test_cmp_greater() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        let args = [int_to_asn(42), int_to_asn(10)];
        let result = kf_asn_cmp(&mut ctx, &args).unwrap();
        assert_eq!(result, LpcValue::Int(1));
    }

    #[test]
    fn test_cmp_negative_vs_positive() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        let args = [int_to_asn(-5), int_to_asn(5)];
        let result = kf_asn_cmp(&mut ctx, &args).unwrap();
        assert_eq!(result, LpcValue::Int(-1));
    }

    #[test]
    fn test_cmp_both_negative() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        let args = [int_to_asn(-10), int_to_asn(-5)];
        let result = kf_asn_cmp(&mut ctx, &args).unwrap();
        assert_eq!(result, LpcValue::Int(-1));
    }

    #[test]
    fn test_zero_operations() {
        let mut ticks = 10000u64;
        let mut ctx = make_ctx(&mut ticks);
        // 0 + 0 mod 10 = 0
        let args = [int_to_asn(0), int_to_asn(0), int_to_asn(10)];
        let result = kf_asn_add(&mut ctx, &args).unwrap();
        assert_eq!(asn_to_int(&result), 0);
    }

    #[test]
    fn test_mag_operations() {
        // Verify internal magnitude arithmetic
        assert_eq!(mag_add(&[1], &[1]), vec![2]);
        assert_eq!(mag_add(&[0xFF], &[1]), vec![1, 0]);
        assert_eq!(mag_sub(&[1, 0], &[1]), vec![0xFF]);
        assert_eq!(mag_mul(&[2], &[3]), vec![6]);
        assert_eq!(mag_divmod(&[10], &[3]), (vec![3], vec![1]));
    }
}
