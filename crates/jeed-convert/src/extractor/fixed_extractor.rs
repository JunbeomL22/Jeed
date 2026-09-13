//! Fixed-width numeric extractor for fixed-format byte slices.

use super::config::Config;
use crate::decimal_core::{
    le_bytes_to_u128, le_bytes_to_u16, le_bytes_to_u32, le_bytes_to_u64, squeeze_point_u128,
    squeeze_point_u16, squeeze_point_u32, squeeze_point_u64,
};
use crate::integer_parser::Biscuit;
use crate::ParseErr;

const PADDED_SIZE_4: usize = 4;
const PADDED_SIZE_8: usize = 8;
const PADDED_SIZE_16: usize = 16;

const I8_MAX: u8 = 127;
const I8_MIN_ABS: u8 = 128;
const I16_MAX: u16 = 32767;
const I16_MIN_ABS: u16 = 32768;
const I32_MAX: u32 = 2147483647;
const I32_MIN_ABS: u32 = 2147483648;
const I64_MAX: u64 = 9223372036854775807;
const I64_MIN_ABS: u64 = 9223372036854775808;
const I128_MAX: u128 = 170141183460469231731687303715884105727;
const I128_MIN_ABS: u128 = 170141183460469231731687303715884105728;

/// Fixed-width extractor for fixed-format numeric data.
///
/// Parses numeric data from byte slices using a configured [`Config`].
#[derive(Debug, Clone, Default, PartialEq, PartialOrd)]
pub struct FixedExtractor {
    /// Configuration for this extractor.
    pub config: Config,
}

impl FixedExtractor {
    /// Creates a new [`FixedExtractor`] with the given config.
    pub fn new(config: Config) -> Self {
        FixedExtractor { config }
    }

    /// Extracts numeric portion of data based on configured indices.
    ///
    /// # Errors
    /// Returns [`ParseErr::InvalidLength`] if data is too short.
    #[inline]
    pub fn clip<'a>(&self, data: &'a [u8]) -> Result<&'a [u8], ParseErr> {
        if data.len() < self.config.numeric_end_idx {
            return Err(ParseErr::InvalidLength);
        }
        Ok(&data[self.config.numeric_start_idx..self.config.numeric_end_idx])
    }

    /// Parses a `u8` from the data slice.
    #[inline]
    pub fn to_u8(&self, data: &[u8]) -> Result<u8, ParseErr> {
        let clipped = self.clip(data)?;
        match self.config.numeric_point_idx {
            None => u8::parse_decimal(clipped),
            Some(idx) => self.parse_with_point_u8(clipped, idx),
        }
    }

    /// Parses a `u16` from the data slice.
    #[inline]
    pub fn to_u16(&self, data: &[u8]) -> Result<u16, ParseErr> {
        let clipped = self.clip(data)?;
        match self.config.numeric_point_idx {
            None => u16::parse_decimal(clipped),
            Some(idx) => self.parse_with_point_u16(clipped, idx),
        }
    }

    /// Parses a `u32` from the data slice.
    #[inline]
    pub fn to_u32(&self, data: &[u8]) -> Result<u32, ParseErr> {
        let clipped = self.clip(data)?;
        match self.config.numeric_point_idx {
            None => u32::parse_decimal(clipped),
            Some(idx) => self.parse_with_point_u32(clipped, idx),
        }
    }

    /// Parses a `u64` from the data slice.
    #[inline]
    pub fn to_u64(&self, data: &[u8]) -> Result<u64, ParseErr> {
        let clipped = self.clip(data)?;
        match self.config.numeric_point_idx {
            None => u64::parse_decimal(clipped),
            Some(idx) => self.parse_with_point_u64(clipped, idx),
        }
    }

    /// Parses a `u128` from the data slice.
    #[inline]
    pub fn to_u128(&self, data: &[u8]) -> Result<u128, ParseErr> {
        let clipped = self.clip(data)?;
        match self.config.numeric_point_idx {
            None => u128::parse_decimal(clipped),
            Some(idx) => self.parse_with_point_u128(clipped, idx),
        }
    }

    /// Parses an `i8` from the data slice.
    #[inline]
    pub fn to_i8(&self, data: &[u8]) -> Result<i8, ParseErr> {
        let clipped = self.clip(data)?;
        if self.is_negative_signed(clipped) {
            let unsigned = self.parse_unsigned_u8(&clipped[1..], true)?;
            if unsigned > I8_MIN_ABS {
                return Err(ParseErr::NegOverflow);
            }
            Ok((!(unsigned as i8)).wrapping_add(1))
        } else {
            let unsigned = self.parse_unsigned_u8(clipped, false)?;
            if unsigned > I8_MAX {
                return Err(ParseErr::Overflow);
            }
            Ok(unsigned as i8)
        }
    }

    /// Parses an `i16` from the data slice.
    #[inline]
    pub fn to_i16(&self, data: &[u8]) -> Result<i16, ParseErr> {
        let clipped = self.clip(data)?;
        if self.is_negative_signed(clipped) {
            let unsigned = self.parse_unsigned_u16(&clipped[1..], true)?;
            if unsigned > I16_MIN_ABS {
                return Err(ParseErr::NegOverflow);
            }
            Ok((!(unsigned as i16)).wrapping_add(1))
        } else {
            let unsigned = self.parse_unsigned_u16(clipped, false)?;
            if unsigned > I16_MAX {
                return Err(ParseErr::Overflow);
            }
            Ok(unsigned as i16)
        }
    }

    /// Parses an `i32` from the data slice.
    #[inline]
    pub fn to_i32(&self, data: &[u8]) -> Result<i32, ParseErr> {
        let clipped = self.clip(data)?;
        if self.is_negative_signed(clipped) {
            let unsigned = self.parse_unsigned_u32(&clipped[1..], true)?;
            if unsigned > I32_MIN_ABS {
                return Err(ParseErr::NegOverflow);
            }
            Ok((!(unsigned as i32)).wrapping_add(1))
        } else {
            let unsigned = self.parse_unsigned_u32(clipped, false)?;
            if unsigned > I32_MAX {
                return Err(ParseErr::Overflow);
            }
            Ok(unsigned as i32)
        }
    }

    /// Parses an `i64` from the data slice.
    #[inline]
    pub fn to_i64(&self, data: &[u8]) -> Result<i64, ParseErr> {
        let clipped = self.clip(data)?;
        if self.is_negative_signed(clipped) {
            let unsigned = self.parse_unsigned_u64(&clipped[1..], true)?;
            if unsigned > I64_MIN_ABS {
                return Err(ParseErr::NegOverflow);
            }
            Ok((!(unsigned as i64)).wrapping_add(1))
        } else {
            let unsigned = self.parse_unsigned_u64(clipped, false)?;
            if unsigned > I64_MAX {
                return Err(ParseErr::Overflow);
            }
            Ok(unsigned as i64)
        }
    }

    /// Parses an `i128` from the data slice.
    #[inline]
    pub fn to_i128(&self, data: &[u8]) -> Result<i128, ParseErr> {
        let clipped = self.clip(data)?;
        if self.is_negative_signed(clipped) {
            let unsigned = self.parse_unsigned_u128(&clipped[1..], true)?;
            if unsigned > I128_MIN_ABS {
                return Err(ParseErr::NegOverflow);
            }
            Ok((!(unsigned as i128)).wrapping_add(1))
        } else {
            let unsigned = self.parse_unsigned_u128(clipped, false)?;
            if unsigned > I128_MAX {
                return Err(ParseErr::Overflow);
            }
            Ok(unsigned as i128)
        }
    }

    /// Parses an `f32` from the data slice.
    #[inline]
    pub fn to_f32(&self, data: &[u8]) -> Result<f32, ParseErr> {
        let int_val = self.to_i64(data)?;
        Ok(int_val as f32 * self.config.fraction_multiplier as f32)
    }

    /// Parses an `f64` from the data slice.
    #[inline]
    pub fn to_f64(&self, data: &[u8]) -> Result<f64, ParseErr> {
        let int_val = self.to_i64(data)?;
        Ok(int_val as f64 * self.config.fraction_multiplier)
    }

    #[inline]
    fn is_negative_signed(&self, clipped: &[u8]) -> bool {
        self.config.is_signed && !clipped.is_empty() && clipped[0] == b'-'
    }

    #[inline]
    fn parse_unsigned_u8(&self, clipped: &[u8], is_after_sign: bool) -> Result<u8, ParseErr> {
        match self.config.numeric_point_idx {
            None => u8::parse_decimal(clipped),
            Some(idx) => {
                let adjusted_idx = if is_after_sign {
                    idx.saturating_sub(1)
                } else {
                    idx
                };
                self.parse_with_point_u8(clipped, adjusted_idx)
            }
        }
    }

    #[inline]
    fn parse_unsigned_u16(&self, clipped: &[u8], is_after_sign: bool) -> Result<u16, ParseErr> {
        match self.config.numeric_point_idx {
            None => u16::parse_decimal(clipped),
            Some(idx) => {
                let adjusted_idx = if is_after_sign {
                    idx.saturating_sub(1)
                } else {
                    idx
                };
                self.parse_with_point_u16(clipped, adjusted_idx)
            }
        }
    }

    #[inline]
    fn parse_unsigned_u32(&self, clipped: &[u8], is_after_sign: bool) -> Result<u32, ParseErr> {
        match self.config.numeric_point_idx {
            None => u32::parse_decimal(clipped),
            Some(idx) => {
                let adjusted_idx = if is_after_sign {
                    idx.saturating_sub(1)
                } else {
                    idx
                };
                self.parse_with_point_u32(clipped, adjusted_idx)
            }
        }
    }

    #[inline]
    fn parse_unsigned_u64(&self, clipped: &[u8], is_after_sign: bool) -> Result<u64, ParseErr> {
        match self.config.numeric_point_idx {
            None => u64::parse_decimal(clipped),
            Some(idx) => {
                let adjusted_idx = if is_after_sign {
                    idx.saturating_sub(1)
                } else {
                    idx
                };
                self.parse_with_point_u64(clipped, adjusted_idx)
            }
        }
    }

    #[inline]
    fn parse_unsigned_u128(&self, clipped: &[u8], is_after_sign: bool) -> Result<u128, ParseErr> {
        match self.config.numeric_point_idx {
            None => u128::parse_decimal(clipped),
            Some(idx) => {
                let adjusted_idx = if is_after_sign {
                    idx.saturating_sub(1)
                } else {
                    idx
                };
                self.parse_with_point_u128(clipped, adjusted_idx)
            }
        }
    }

    #[inline]
    fn parse_with_point_u8(&self, clipped: &[u8], point_idx: usize) -> Result<u8, ParseErr> {
        let length = clipped.len();
        if length == 0 {
            return Err(ParseErr::Empty);
        }
        if length == 2 {
            let chunk = le_bytes_to_u16(clipped);
            let squeezed = squeeze_point_u16(chunk, point_idx)?;
            let bytes = squeezed.to_le_bytes();
            u8::parse_decimal(&bytes[1..])
        } else if length == PADDED_SIZE_4 {
            let chunk = le_bytes_to_u32(clipped);
            let squeezed = squeeze_point_u32(chunk, point_idx)?;
            let bytes = squeezed.to_le_bytes();
            u8::parse_decimal(&bytes[1..])
        } else {
            self.parse_with_point_generic_u8(clipped, point_idx)
        }
    }

    #[inline]
    fn parse_with_point_generic_u8(&self, clipped: &[u8], point_idx: usize) -> Result<u8, ParseErr> {
        let length = clipped.len();
        if length <= PADDED_SIZE_4 {
            let mut padded = [b'0'; PADDED_SIZE_4];
            let start = PADDED_SIZE_4 - length;
            padded[start..].copy_from_slice(clipped);
            let chunk = le_bytes_to_u32(&padded);
            let adjusted_idx = point_idx + start;
            let squeezed = squeeze_point_u32(chunk, adjusted_idx)?;
            let bytes = squeezed.to_le_bytes();
            u8::parse_decimal(&bytes[start..])
        } else {
            Err(ParseErr::Overflow)
        }
    }

    #[inline]
    fn parse_with_point_u16(&self, clipped: &[u8], point_idx: usize) -> Result<u16, ParseErr> {
        let length = clipped.len();
        if length == 0 {
            return Err(ParseErr::Empty);
        }
        if length <= PADDED_SIZE_4 {
            let mut padded = [b'0'; PADDED_SIZE_4];
            let start = PADDED_SIZE_4 - length;
            padded[start..].copy_from_slice(clipped);
            let chunk = le_bytes_to_u32(&padded);
            let adjusted_idx = point_idx + start;
            let squeezed = squeeze_point_u32(chunk, adjusted_idx)?;
            let bytes = squeezed.to_le_bytes();
            u16::parse_decimal(&bytes[start..])
        } else if length <= PADDED_SIZE_8 {
            let mut padded = [b'0'; PADDED_SIZE_8];
            let start = PADDED_SIZE_8 - length;
            padded[start..].copy_from_slice(clipped);
            let chunk = le_bytes_to_u64(&padded);
            let adjusted_idx = point_idx + start;
            let squeezed = squeeze_point_u64(chunk, adjusted_idx)?;
            let bytes = squeezed.to_le_bytes();
            u16::parse_decimal(&bytes[start..])
        } else {
            Err(ParseErr::Overflow)
        }
    }

    #[inline]
    fn parse_with_point_u32(&self, clipped: &[u8], point_idx: usize) -> Result<u32, ParseErr> {
        let length = clipped.len();
        if length == 0 {
            return Err(ParseErr::Empty);
        }
        if length <= PADDED_SIZE_8 {
            let mut padded = [b'0'; PADDED_SIZE_8];
            let start = PADDED_SIZE_8 - length;
            padded[start..].copy_from_slice(clipped);
            let chunk = le_bytes_to_u64(&padded);
            let adjusted_idx = point_idx + start;
            let squeezed = squeeze_point_u64(chunk, adjusted_idx)?;
            let bytes = squeezed.to_le_bytes();
            u32::parse_decimal(&bytes[start..])
        } else if length <= PADDED_SIZE_16 {
            let mut padded = [b'0'; PADDED_SIZE_16];
            let start = PADDED_SIZE_16 - length;
            padded[start..].copy_from_slice(clipped);
            let chunk = le_bytes_to_u128(&padded);
            let adjusted_idx = point_idx + start;
            let squeezed = squeeze_point_u128(chunk, adjusted_idx)?;
            let bytes = squeezed.to_le_bytes();
            u32::parse_decimal(&bytes[start..])
        } else {
            Err(ParseErr::Overflow)
        }
    }

    #[inline]
    fn parse_with_point_u64(&self, clipped: &[u8], point_idx: usize) -> Result<u64, ParseErr> {
        let length = clipped.len();
        if length == 0 {
            return Err(ParseErr::Empty);
        }
        if length <= PADDED_SIZE_8 {
            let mut padded = [b'0'; PADDED_SIZE_8];
            let start = PADDED_SIZE_8 - length;
            padded[start..].copy_from_slice(clipped);
            let chunk = le_bytes_to_u64(&padded);
            let adjusted_idx = point_idx + start;
            let squeezed = squeeze_point_u64(chunk, adjusted_idx)?;
            let bytes = squeezed.to_le_bytes();
            u64::parse_decimal(&bytes[start..])
        } else if length <= PADDED_SIZE_16 {
            let mut padded = [b'0'; PADDED_SIZE_16];
            let start = PADDED_SIZE_16 - length;
            padded[start..].copy_from_slice(clipped);
            let chunk = le_bytes_to_u128(&padded);
            let adjusted_idx = point_idx + start;
            let squeezed = squeeze_point_u128(chunk, adjusted_idx)?;
            let bytes = squeezed.to_le_bytes();
            u64::parse_decimal(&bytes[start..])
        } else {
            Err(ParseErr::Overflow)
        }
    }

    #[inline]
    fn parse_with_point_u128(&self, clipped: &[u8], point_idx: usize) -> Result<u128, ParseErr> {
        let length = clipped.len();
        if length == 0 {
            return Err(ParseErr::Empty);
        }
        if length <= PADDED_SIZE_16 {
            let mut padded = [b'0'; PADDED_SIZE_16];
            let start = PADDED_SIZE_16 - length;
            padded[start..].copy_from_slice(clipped);
            let chunk = le_bytes_to_u128(&padded);
            let adjusted_idx = point_idx + start;
            let squeezed = squeeze_point_u128(chunk, adjusted_idx)?;
            let bytes = squeezed.to_le_bytes();
            u128::parse_decimal(&bytes[start..])
        } else {
            Err(ParseErr::Overflow)
        }
    }
}
