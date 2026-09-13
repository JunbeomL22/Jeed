//! Checked byte-to-integer conversion with validation.

use super::{
    check_decimal_bit_u128_optimal, check_decimal_bit_u16_optimal, check_decimal_bit_u32_optimal,
    check_decimal_bit_u64_optimal, eight_to_u64, four_to_u32, le_bytes_to_u128, le_bytes_to_u16,
    le_bytes_to_u32, le_bytes_to_u64, sixteen_to_u128, two_to_u16_decimal,
};
use crate::ParseErr;

/// Converts a single ASCII digit byte to `u8`.
///
/// # Errors
/// Returns [`ParseErr::Empty`] if input is empty.
/// Returns [`ParseErr::InvalidDigit`] if byte is not an ASCII digit.
#[inline]
pub fn checked_conversion_u8(input: &[u8]) -> Result<u8, ParseErr> {
    let byte = *input.first().ok_or(ParseErr::Empty)?;
    if (0x30..=0x39).contains(&byte) {
        Ok(byte - 0x30)
    } else {
        Err(ParseErr::InvalidDigit)
    }
}

/// Converts a 2-byte ASCII digit slice to `u16`.
///
/// # Errors
/// Returns [`ParseErr::InvalidDigit`] if any byte is not an ASCII digit.
#[inline]
pub fn checked_conversion_u16(input: &[u8]) -> Result<u16, ParseErr> {
    let chunk = le_bytes_to_u16(input);
    if check_decimal_bit_u16_optimal(chunk) {
        Ok(two_to_u16_decimal(chunk))
    } else {
        Err(ParseErr::InvalidDigit)
    }
}

/// Converts a 4-byte ASCII digit slice to `u32`.
///
/// # Errors
/// Returns [`ParseErr::InvalidDigit`] if any byte is not an ASCII digit.
#[inline]
pub fn checked_conversion_u32(input: &[u8]) -> Result<u32, ParseErr> {
    let chunk = le_bytes_to_u32(input);
    if check_decimal_bit_u32_optimal(chunk) {
        Ok(four_to_u32(chunk))
    } else {
        Err(ParseErr::InvalidDigit)
    }
}

/// Converts an 8-byte ASCII digit slice to `u64`.
///
/// # Errors
/// Returns [`ParseErr::InvalidDigit`] if any byte is not an ASCII digit.
#[inline]
pub fn checked_conversion_u64(input: &[u8]) -> Result<u64, ParseErr> {
    let chunk = le_bytes_to_u64(input);
    if check_decimal_bit_u64_optimal(chunk) {
        Ok(eight_to_u64(chunk))
    } else {
        Err(ParseErr::InvalidDigit)
    }
}

/// Converts a 16-byte ASCII digit slice to `u128`.
///
/// # Errors
/// Returns [`ParseErr::InvalidDigit`] if any byte is not an ASCII digit.
#[inline]
pub fn checked_conversion_u128(input: &[u8]) -> Result<u128, ParseErr> {
    let chunk = le_bytes_to_u128(input);
    if check_decimal_bit_u128_optimal(chunk) {
        Ok(sixteen_to_u128(chunk))
    } else {
        Err(ParseErr::InvalidDigit)
    }
}
