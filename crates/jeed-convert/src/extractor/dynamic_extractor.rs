//! Dynamic decimal extractor for variable-length inputs.
//!
//! Parses decimal strings and adjusts to target decimal places.

use crate::ParseErr;
use jeed_wire::Scale;

/// Dynamic decimal extractor for variable-length decimal strings.
///
/// Unlike [`FixedExtractor`](super::FixedExtractor), handles variable-length input
/// and adjusts values to a target number of decimal places.
#[derive(Debug, Clone, Default, PartialEq, PartialOrd)]
pub struct DynamicExtractor {
    /// Number of decimal places in the result.
    pub target_decimals: usize,
}

impl DynamicExtractor {
    /// Creates a new [`DynamicExtractor`] with target decimal places.
    ///
    /// # Example
    /// ```
    /// use jeed_convert::extractor::DynamicExtractor;
    ///
    /// let extractor = DynamicExtractor::new(4);
    /// assert_eq!(extractor.parse(b"123.45"), 1234500);
    /// ```
    pub fn new(target_decimals: usize) -> Self {
        Self { target_decimals }
    }

    /// Returns [`Scale`] for this extractor's decimal places, or `None` if > 8.
    #[inline]
    pub fn scale(&self) -> Option<Scale> {
        Scale::from_decimals(self.target_decimals)
    }

    /// Parses a decimal string and returns `i64` with target decimal places.
    ///
    /// Supports negative numbers with leading `-` and optional `+` for positive.
    /// Empty input returns 0.
    ///
    /// # Example
    /// ```
    /// use jeed_convert::extractor::DynamicExtractor;
    ///
    /// let extractor = DynamicExtractor::new(4);
    /// assert_eq!(extractor.parse(b"123.45"), 1234500);
    /// assert_eq!(extractor.parse(b"-123.45"), -1234500);
    /// ```
    pub fn parse(&self, bytes: &[u8]) -> i64 {
        if bytes.is_empty() {
            return 0;
        }

        let (is_negative, numeric_bytes) = match bytes[0] {
            b'-' => (true, &bytes[1..]),
            b'+' => (false, &bytes[1..]),
            _ => (false, bytes),
        };

        let dot_pos = self.find_decimal_point(numeric_bytes);

        let result = match dot_pos {
            Some(pos) => self.parse_with_decimal_point(numeric_bytes, pos),
            None => self.parse_integer_only(numeric_bytes),
        };

        if is_negative { -result } else { result }
    }

    /// Attempts to parse a decimal string, returning `None` if invalid.
    ///
    /// Unlike [`parse`](Self::parse), validates input and returns `None` for invalid data.
    ///
    /// # Example
    /// ```
    /// use jeed_convert::extractor::DynamicExtractor;
    ///
    /// let extractor = DynamicExtractor::new(2);
    /// assert_eq!(extractor.try_parse(b"123.45"), Some(12345));
    /// assert_eq!(extractor.try_parse(b"abc"), None);
    /// assert_eq!(extractor.try_parse(b""), None);
    /// ```
    pub fn try_parse(&self, bytes: &[u8]) -> Option<i64> {
        if bytes.is_empty() {
            return None;
        }

        let (is_negative, numeric_bytes) = match bytes[0] {
            b'-' => (true, &bytes[1..]),
            b'+' => (false, &bytes[1..]),
            _ => (false, bytes),
        };

        if numeric_bytes.is_empty() {
            return None;
        }

        let mut decimal_count = 0;
        let mut digit_count = 0;
        for &b in numeric_bytes {
            if b == b'.' {
                decimal_count += 1;
                if decimal_count > 1 {
                    return None;
                }
            } else if b.is_ascii_digit() {
                digit_count += 1;
            } else {
                return None;
            }
        }

        if digit_count == 0 {
            return None;
        }

        let dot_pos = self.find_decimal_point(numeric_bytes);

        let result = match dot_pos {
            Some(pos) => self.parse_with_decimal_point(numeric_bytes, pos),
            None => self.parse_integer_only(numeric_bytes),
        };

        Some(if is_negative { -result } else { result })
    }

    /// Parses a `u8` from the data slice.
    #[inline]
    pub fn to_u8(&self, data: &[u8]) -> Result<u8, ParseErr> {
        let val = self.try_parse(data).ok_or(ParseErr::InvalidDigit)?;
        if val < 0 {
            Err(ParseErr::NegOverflow)
        } else if val > u8::MAX as i64 {
            Err(ParseErr::Overflow)
        } else {
            Ok(val as u8)
        }
    }

    /// Parses a `u16` from the data slice.
    #[inline]
    pub fn to_u16(&self, data: &[u8]) -> Result<u16, ParseErr> {
        let val = self.try_parse(data).ok_or(ParseErr::InvalidDigit)?;
        if val < 0 {
            Err(ParseErr::NegOverflow)
        } else if val > u16::MAX as i64 {
            Err(ParseErr::Overflow)
        } else {
            Ok(val as u16)
        }
    }

    /// Parses a `u32` from the data slice.
    #[inline]
    pub fn to_u32(&self, data: &[u8]) -> Result<u32, ParseErr> {
        let val = self.try_parse(data).ok_or(ParseErr::InvalidDigit)?;
        if val < 0 {
            Err(ParseErr::NegOverflow)
        } else if val > u32::MAX as i64 {
            Err(ParseErr::Overflow)
        } else {
            Ok(val as u32)
        }
    }

    /// Parses a `u64` from the data slice.
    #[inline]
    pub fn to_u64(&self, data: &[u8]) -> Result<u64, ParseErr> {
        let val = self.try_parse(data).ok_or(ParseErr::InvalidDigit)?;
        if val < 0 {
            Err(ParseErr::NegOverflow)
        } else {
            Ok(val as u64)
        }
    }

    /// Parses a `u128` from the data slice.
    #[inline]
    pub fn to_u128(&self, data: &[u8]) -> Result<u128, ParseErr> {
        let val = self.try_parse(data).ok_or(ParseErr::InvalidDigit)?;
        if val < 0 {
            Err(ParseErr::NegOverflow)
        } else {
            Ok(val as u128)
        }
    }

    /// Parses an `i8` from the data slice.
    #[inline]
    pub fn to_i8(&self, data: &[u8]) -> Result<i8, ParseErr> {
        let val = self.try_parse(data).ok_or(ParseErr::InvalidDigit)?;
        if val < i8::MIN as i64 {
            Err(ParseErr::NegOverflow)
        } else if val > i8::MAX as i64 {
            Err(ParseErr::Overflow)
        } else {
            Ok(val as i8)
        }
    }

    /// Parses an `i16` from the data slice.
    #[inline]
    pub fn to_i16(&self, data: &[u8]) -> Result<i16, ParseErr> {
        let val = self.try_parse(data).ok_or(ParseErr::InvalidDigit)?;
        if val < i16::MIN as i64 {
            Err(ParseErr::NegOverflow)
        } else if val > i16::MAX as i64 {
            Err(ParseErr::Overflow)
        } else {
            Ok(val as i16)
        }
    }

    /// Parses an `i32` from the data slice.
    #[inline]
    pub fn to_i32(&self, data: &[u8]) -> Result<i32, ParseErr> {
        let val = self.try_parse(data).ok_or(ParseErr::InvalidDigit)?;
        if val < i32::MIN as i64 {
            Err(ParseErr::NegOverflow)
        } else if val > i32::MAX as i64 {
            Err(ParseErr::Overflow)
        } else {
            Ok(val as i32)
        }
    }

    /// Parses an `i64` from the data slice.
    #[inline]
    pub fn to_i64(&self, data: &[u8]) -> Result<i64, ParseErr> {
        self.try_parse(data).ok_or(ParseErr::InvalidDigit)
    }

    /// Parses an `i128` from the data slice.
    #[inline]
    pub fn to_i128(&self, data: &[u8]) -> Result<i128, ParseErr> {
        let val = self.try_parse(data).ok_or(ParseErr::InvalidDigit)?;
        Ok(val as i128)
    }

    /// Parses an `f32` from the data slice.
    #[inline]
    pub fn to_f32(&self, data: &[u8]) -> Result<f32, ParseErr> {
        let val = self.try_parse(data).ok_or(ParseErr::InvalidDigit)?;
        let divisor = 10_f32.powi(self.target_decimals as i32);
        Ok(val as f32 / divisor)
    }

    /// Parses an `f64` from the data slice.
    #[inline]
    pub fn to_f64(&self, data: &[u8]) -> Result<f64, ParseErr> {
        let val = self.try_parse(data).ok_or(ParseErr::InvalidDigit)?;
        let divisor = 10_f64.powi(self.target_decimals as i32);
        Ok(val as f64 / divisor)
    }

    #[inline]
    fn find_decimal_point(&self, bytes: &[u8]) -> Option<usize> {
        bytes.iter().position(|&b| b == b'.')
    }

    #[inline]
    fn parse_with_decimal_point(&self, bytes: &[u8], decimal_pos: usize) -> i64 {
        let integer_part = self.parse_digits(&bytes[..decimal_pos]);
        let (decimal_part, decimal_digit_count) = self.parse_decimal_digits(&bytes[decimal_pos + 1..]);

        self.adjust_to_target_decimals(integer_part, decimal_part, decimal_digit_count)
    }

    #[inline]
    fn parse_integer_only(&self, bytes: &[u8]) -> i64 {
        let integer_part = self.parse_digits(bytes);
        let multiplier = 10_i64.pow(self.target_decimals as u32);
        integer_part * multiplier
    }

    #[inline]
    fn parse_digits(&self, bytes: &[u8]) -> i64 {
        let mut result: i64 = 0;
        for &b in bytes {
            if b.is_ascii_digit() {
                result = result * 10 + (b - b'0') as i64;
            }
        }
        result
    }

    #[inline]
    fn parse_decimal_digits(&self, bytes: &[u8]) -> (i64, usize) {
        let mut result: i64 = 0;
        let mut digit_count = 0;
        for &b in bytes {
            if b.is_ascii_digit() {
                result = result * 10 + (b - b'0') as i64;
                digit_count += 1;
            }
        }
        (result, digit_count)
    }

    #[inline]
    fn adjust_to_target_decimals(
        &self,
        integer_part: i64,
        decimal_part: i64,
        decimal_digit_count: usize,
    ) -> i64 {
        let multiplier = 10_i64.pow(self.target_decimals as u32);

        if decimal_digit_count < self.target_decimals {
            let scale = 10_i64.pow((self.target_decimals - decimal_digit_count) as u32);
            integer_part * multiplier + decimal_part * scale
        } else if decimal_digit_count > self.target_decimals {
            let scale = 10_i64.pow((decimal_digit_count - self.target_decimals) as u32);
            integer_part * multiplier + decimal_part / scale
        } else {
            integer_part * multiplier + decimal_part
        }
    }
}
