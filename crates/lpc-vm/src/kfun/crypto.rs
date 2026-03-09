//! Hash and crypto kfuns: crypt, hash_crc16, hash_crc32, hash_string, encrypt, decrypt.

use crate::bytecode::LpcValue;
use super::{KfunContext, LpcError, require_string, require_int};

// CRC-16/CCITT lookup table
const CRC16_TABLE: [u16; 256] = {
    let mut table = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = (i as u16) << 8;
        let mut j = 0;
        while j < 8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

// CRC-32 lookup table (standard polynomial 0xEDB88320)
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// crypt(string password, string salt) -> string
///
/// Unix-style password hashing.
///
/// TODO: Full implementation requires a crypt library (e.g., pwhash or libc crypt).
/// Currently returns a stub hash.
pub fn kf_crypt(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let _password = require_string(&args[0], 0)?;
    let _salt = require_string(&args[1], 1)?;
    // TODO: Implement actual Unix crypt(3) hashing
    Err(LpcError::RuntimeError(
        "crypt: not yet implemented (requires crypt library)".into(),
    ))
}

/// hash_crc16(string data, ...) -> int
///
/// CRC-16/CCITT checksum. Multiple string arguments are accumulated.
pub fn kf_hash_crc16(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let mut crc: u16 = 0xFFFF;
    for (i, arg) in args.iter().enumerate() {
        let data = require_string(arg, i)?;
        for byte in data.bytes() {
            crc = (crc << 8) ^ CRC16_TABLE[((crc >> 8) as u8 ^ byte) as usize];
        }
    }
    Ok(LpcValue::Int(crc as i64))
}

/// hash_crc32(string data, ...) -> int
///
/// CRC-32 checksum. Multiple string arguments are accumulated.
pub fn kf_hash_crc32(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let mut crc: u32 = 0xFFFF_FFFF;
    for (i, arg) in args.iter().enumerate() {
        let data = require_string(arg, i)?;
        for byte in data.bytes() {
            crc = (crc >> 8) ^ CRC32_TABLE[(crc as u8 ^ byte) as usize];
        }
    }
    Ok(LpcValue::Int(!crc as i32 as i64))
}

/// hash_string(string key, int table_size) -> int
///
/// Hash string to integer in range 0..table_size-1.
/// Uses DJB2 hash algorithm.
pub fn kf_hash_string(_ctx: &mut KfunContext, args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    let key = require_string(&args[0], 0)?;
    let table_size = require_int(&args[1], 1)?;
    if table_size <= 0 {
        return Err(LpcError::ValueError(
            "table_size must be positive".into(),
        ));
    }

    // DJB2 hash
    let mut hash: u64 = 5381;
    for byte in key.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(byte as u64);
    }

    Ok(LpcValue::Int((hash % table_size as u64) as i64))
}

/// encrypt(string data, string key, varargs string cipher) -> string
///
/// Encrypt data using the specified cipher (or default cipher).
pub fn kf_encrypt(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "encrypt: cipher not yet implemented".into(),
    ))
}

/// decrypt(string data, string key, varargs string cipher) -> string
///
/// Decrypt data using the specified cipher (or default cipher).
pub fn kf_decrypt(_ctx: &mut KfunContext, _args: &[LpcValue]) -> Result<LpcValue, LpcError> {
    Err(LpcError::RuntimeError(
        "decrypt: cipher not yet implemented".into(),
    ))
}
