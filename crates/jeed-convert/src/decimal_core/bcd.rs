//! BCD (Binary-Coded Decimal) to binary conversion using SWAR.
//!
//! Each function processes N packed BCD digits stored in a single integer,
//! using SWAR (SIMD Within A Register) bit manipulation to convert them to
//! their binary value in a fixed number of steps (O(log N)).
//!
//! # Input Requirements
//!
//! Each byte (or pair of nibbles) in the input must contain a valid BCD digit
//! pair — i.e., each nibble must be in `0x0..=0x9`. Passing values with
//! nibbles outside `0..=9` produces undefined (but not unsafe) numeric results.
//!
//! # Valid Input Ranges
//!
//! | Function | Input type | Max valid BCD value |
//! |----------|-----------|---------------------|
//! | [`two_to_u16_decimal`] | `u16` | `0x0909` (99) |
//! | [`four_to_u32`] | `u32` | `0x09090909` (9999) |
//! | [`eight_to_u64`] | `u64` | `0x0909090909090909` (99_999_999) |
//! | [`sixteen_to_u128`] | `u128` | 16 × `0x09` nibble pairs (9_999_999_999_999_999) |

/// Converts two packed BCD digits to `u16`.
#[inline]
#[must_use]
pub fn two_to_u16_decimal(chunk: u16) -> u16 {
    ((chunk & 0x0f00) >> 8) + (chunk & 0x000f) * 10
}

/// Converts four packed BCD digits to `u32`.
#[inline]
#[must_use]
pub fn four_to_u32(mut chunk: u32) -> u32 {
    let lower_digits = (chunk & 0x0f000f00) >> 8;
    let upper_digits = (chunk & 0x000f000f) * 10;
    chunk = lower_digits + upper_digits;

    let lower_digits = (chunk & 0x00ff0000) >> 16;
    let upper_digits = (chunk & 0x000000ff) * 100;
    chunk = lower_digits + upper_digits;

    chunk
}

/// Converts eight packed BCD digits to `u64`.
#[inline]
#[must_use]
pub fn eight_to_u64(mut chunk: u64) -> u64 {
    let lower_digits = (chunk & 0x0f000f000f000f00) >> 8;
    let upper_digits = (chunk & 0x000f000f000f000f) * 10;
    chunk = lower_digits + upper_digits;

    let lower_digits = (chunk & 0x00ff000000ff0000) >> 16;
    let upper_digits = (chunk & 0x000000ff000000ff) * 100;
    chunk = lower_digits + upper_digits;

    let lower_digits = (chunk & 0x0000ffff00000000) >> 32;
    let upper_digits = (chunk & 0x000000000000ffff) * 10000;
    chunk = lower_digits + upper_digits;

    chunk
}

/// Converts sixteen packed BCD digits to `u128`.
#[inline]
#[must_use]
pub fn sixteen_to_u128(mut chunk: u128) -> u128 {
    let lower_digits = (chunk & 0x0f000f000f000f000f000f000f000f00) >> 8;
    let upper_digits = (chunk & 0x000f000f000f000f000f000f000f000f) * 10;
    chunk = lower_digits + upper_digits;

    let lower_digits = (chunk & 0x00ff000000ff000000ff000000ff0000) >> 16;
    let upper_digits = (chunk & 0x000000ff000000ff000000ff000000ff) * 100;
    chunk = lower_digits + upper_digits;

    let lower_digits = (chunk & 0x0000ffff000000000000ffff00000000) >> 32;
    let upper_digits = (chunk & 0x000000000000ffff000000000000ffff) * 10000;
    chunk = lower_digits + upper_digits;

    let lower_digits = (chunk & 0x00000000ffffffff0000000000000000) >> 64;
    let upper_digits = (chunk & 0x000000000000000000000000ffffffff) * 100_000_000;
    chunk = lower_digits + upper_digits;

    chunk
}
