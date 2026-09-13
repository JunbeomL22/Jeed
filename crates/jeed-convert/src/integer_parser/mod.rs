//! Fast decimal integer parser using bitwise operations.
//!
//! The `Biscuit` trait provides `parse_decimal` for all integer types.
//!
//! # Example
//! ```
//! use jeed_convert::ParseErr;
//! use jeed_convert::integer_parser::Biscuit;
//!
//! let val = i32::parse_decimal(b"1234").unwrap();
//! assert_eq!(val, 1234);
//!
//! let err = i32::parse_decimal(b"abc");
//! assert_eq!(err, Err(ParseErr::InvalidDigit));
//! ```

/// Signed integer decimal parsing implementation.
pub mod integer_decimal;

/// Unsigned integer decimal parsing implementation.
pub mod unsigned_decimal;

use crate::ParseErr;

/// Parser trait for decimal notation.
///
/// Does not support scientific notation.
pub trait Biscuit: Sized {
    /// Parses a decimal integer from bytes.
    #[inline]
    fn parse_decimal(u: &[u8]) -> Result<Self, ParseErr> {
        Self::unsigned_decimal_core(u, false, false)
    }

    #[doc(hidden)]
    fn unsigned_decimal_core(
        _u: &[u8],
        _neg_max_check: bool,
        _pos_max_check: bool,
    ) -> Result<Self, ParseErr> {
        unimplemented!("This function should be implemented in the child struct")
    }
}
