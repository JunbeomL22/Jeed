//! ASCII digit validation using SWAR (SIMD Within A Register).

/// Checks if all bytes in `u16` are ASCII digits (`'0'` to `'9'`).
#[inline]
#[must_use]
pub fn check_decimal_bit_u16_optimal(chunk: u16) -> bool {
    let not_above = chunk.wrapping_sub(0x3A3A);
    let not_below = 0x2F2F_u16.wrapping_sub(chunk);
    (not_above & not_below & 0x8080) == 0x8080
}

/// Checks if all bytes in `u32` are ASCII digits.
#[inline]
#[must_use]
pub fn check_decimal_bit_u32_optimal(chunk: u32) -> bool {
    let not_above = chunk.wrapping_sub(0x3A3A3A3A);
    let not_below = 0x2F2F2F2F_u32.wrapping_sub(chunk);
    (not_above & not_below & 0x80808080) == 0x80808080
}

/// Checks if all bytes in `u64` are ASCII digits.
#[inline]
#[must_use]
pub fn check_decimal_bit_u64_optimal(chunk: u64) -> bool {
    let not_above = chunk.wrapping_sub(0x3A3A3A3A3A3A3A3A);
    let not_below = 0x2F2F2F2F2F2F2F2F_u64.wrapping_sub(chunk);
    (not_above & not_below & 0x8080808080808080) == 0x8080808080808080
}

/// Checks if all bytes in `u128` are ASCII digits.
#[inline]
#[must_use]
pub fn check_decimal_bit_u128_optimal(chunk: u128) -> bool {
    let not_above = chunk.wrapping_sub(0x3A3A3A3A3A3A3A3A3A3A3A3A3A3A3A3A);
    let not_below = 0x2F2F2F2F2F2F2F2F2F2F2F2F2F2F2F2F_u128.wrapping_sub(chunk);
    (not_above & not_below & 0x80808080808080808080808080808080)
        == 0x80808080808080808080808080808080
}

const ZERO_COMPLEMENT_U64: u64 = 0x00CF00CF00CF00CF;
const NINE_COMPLEMENT_U64: u64 = 0x00C600C600C600C6;
const CHECKER_MASK_U64: u64 = 0xFF00FF00FF00FF00;

/// Checks if all bytes in `u64` are ASCII digits using pair-based validation.
#[inline]
#[must_use]
pub fn check_decimal_bit_u64(chunk: u64) -> bool {
    let lower_upper_check =
        (((chunk & 0x00FF00FF00FF00FF) + NINE_COMPLEMENT_U64) & CHECKER_MASK_U64) == 0;
    let lower_lower_check = (((0x00FF00FF00FF00FF - (chunk & 0x00FF00FF00FF00FF))
        + ZERO_COMPLEMENT_U64)
        & CHECKER_MASK_U64)
        != 0;

    let upper_upper_check =
        ((((chunk & 0xFF00FF00FF00FF00) >> 8) + NINE_COMPLEMENT_U64) & CHECKER_MASK_U64) == 0;
    let upper_lower_check = (((0xFF00FF00FF00FF00 - ((chunk & 0xFF00FF00FF00FF00) >> 8))
        + ZERO_COMPLEMENT_U64)
        & CHECKER_MASK_U64)
        != 0;

    lower_upper_check && lower_lower_check && upper_upper_check && upper_lower_check
}
