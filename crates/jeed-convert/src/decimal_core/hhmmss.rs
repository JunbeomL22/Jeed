//! HHMMSS time format parser for market data timestamps.

use crate::decimal_core::{check_decimal_bit_u64, le_bytes_to_u64};
use crate::ParseErr;

/// Parses HHMMSS (6 bytes) or HHMMSSCC (8 bytes) format to total seconds.
///
/// # Errors
/// Returns [`ParseErr::InvalidLength`] if input length is not 6 or 8 bytes.
/// Returns [`ParseErr::InvalidDigit`] if any byte is not an ASCII digit.
#[inline]
pub fn parse_hhmmss_to_seconds(u: &[u8]) -> Result<u64, ParseErr> {
    if u.len() != 6 && u.len() != 8 {
        return Err(ParseErr::InvalidLength);
    }
    let mut res = le_bytes_to_u64(u);

    if !check_decimal_bit_u64(res) {
        return Err(ParseErr::InvalidDigit);
    }

    let lower_digits = (res & 0x0f000f000f000f00) >> 8;
    let upper_digits = (res & 0x000f000f000f000f) * 10;
    res = lower_digits + upper_digits;

    let lower_digits = (res & 0x00ff000000ff0000) >> 16;
    let upper_digits = (res & 0x000000ff000000ff) * 60;
    res = lower_digits + upper_digits;

    let lower_digits = (res & 0x0000ffff00000000) >> 32;
    let upper_digits = (res & 0x000000000000ffff) * 3_600;
    res = lower_digits + upper_digits;

    Ok(res)
}
