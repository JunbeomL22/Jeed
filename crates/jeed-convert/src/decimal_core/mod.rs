//! Low-level decimal conversion primitives using SWAR techniques.
//!
//! High-performance functions for BCD conversion, digit validation,
//! byte slice conversion, decimal point removal, and checked conversion.

/// Binary-coded decimal (BCD) to integer conversion.
pub mod bcd;

/// Bitwise digit validation for packed integer representations.
pub mod check;

/// Checked decimal-to-integer conversion with overflow detection.
pub mod checked_conversion;

/// HHMMSS time string to integer conversion.
pub mod hhmmss;

/// Little-endian byte slice to unsigned integer conversion.
pub mod le_bytes;

/// Decimal point removal from packed integer representations.
pub mod squeeze_point;

/// Re-exports little-endian byte-to-integer converters.
pub use le_bytes::{le_bytes_to_u128, le_bytes_to_u16, le_bytes_to_u32, le_bytes_to_u64};

/// Re-exports BCD-to-integer converters.
pub use bcd::{eight_to_u64, four_to_u32, sixteen_to_u128, two_to_u16_decimal};

/// Re-exports bitwise digit validation functions.
pub use check::{
    check_decimal_bit_u16_optimal, check_decimal_bit_u32_optimal, check_decimal_bit_u64,
    check_decimal_bit_u64_optimal, check_decimal_bit_u128_optimal,
};

/// Re-exports decimal point removal functions.
pub use squeeze_point::{
    squeeze_point_u128, squeeze_point_u16, squeeze_point_u32, squeeze_point_u64,
};

/// Re-exports checked decimal-to-integer converters.
pub use checked_conversion::{
    checked_conversion_u128, checked_conversion_u16, checked_conversion_u32,
    checked_conversion_u64, checked_conversion_u8,
};
