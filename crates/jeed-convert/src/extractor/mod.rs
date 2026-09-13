//! Fixed-width and variable-length numeric field extractors.
//!
//! [`FixedExtractor`] is the one the KRX decoders use: KRX fields are fixed
//! width with the decimal point (if any) always in the same place, so the point
//! index is configuration rather than something to discover per message.
//!
//! [`DynamicExtractor`] handles variable-length decimal text — FIX tag values,
//! where `100`, `100.0` and `100.00` all arrive on the same tag.

pub mod config;
pub mod dynamic_extractor;
pub mod fixed_extractor;

pub use config::Config;
pub use dynamic_extractor::DynamicExtractor;
pub use fixed_extractor::FixedExtractor;

/// Re-exported so an extractor's errors are reachable from where it is.
pub use crate::error::{ConfigErr, ParseErr};

use jeed_wire::Scale;

/// A numeric field reader, fixed-width or variable-length.
#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub enum Extractor {
    /// Fixed-width field with a configured point index.
    Fixed(FixedExtractor),

    /// Variable-length decimal text normalised to a target scale.
    Dynamic(DynamicExtractor),
}

impl Default for Extractor {
    fn default() -> Self {
        Self::Fixed(FixedExtractor::default())
    }
}

impl From<FixedExtractor> for Extractor {
    fn from(ext: FixedExtractor) -> Self {
        Self::Fixed(ext)
    }
}

impl From<DynamicExtractor> for Extractor {
    fn from(ext: DynamicExtractor) -> Self {
        Self::Dynamic(ext)
    }
}

impl Extractor {
    /// Fixed-width extractor from a built [`Config`].
    #[inline]
    pub fn new_fixed(config: Config) -> Self {
        Self::Fixed(FixedExtractor::new(config))
    }

    /// Variable-length extractor normalising to `target_decimals` places.
    #[inline]
    pub fn new_dynamic(target_decimals: usize) -> Self {
        Self::Dynamic(DynamicExtractor::new(target_decimals))
    }

    /// Total field width in bytes; zero for a dynamic extractor, whose width
    /// is whatever the message happened to send.
    #[inline]
    #[must_use]
    pub fn get_total_size(&self) -> usize {
        match self {
            Self::Fixed(ext) => ext.config.total_size,
            Self::Dynamic(_) => 0,
        }
    }

    /// Parses a `u8` from the data slice.
    #[inline]
    pub fn to_u8(&self, data: &[u8]) -> Result<u8, ParseErr> {
        match self {
            Extractor::Fixed(ext) => ext.to_u8(data),
            Extractor::Dynamic(ext) => ext.to_u8(data),
        }
    }

    /// Parses a `u16` from the data slice.
    #[inline]
    pub fn to_u16(&self, data: &[u8]) -> Result<u16, ParseErr> {
        match self {
            Extractor::Fixed(ext) => ext.to_u16(data),
            Extractor::Dynamic(ext) => ext.to_u16(data),
        }
    }

    /// Parses a `u32` from the data slice.
    #[inline]
    pub fn to_u32(&self, data: &[u8]) -> Result<u32, ParseErr> {
        match self {
            Extractor::Fixed(ext) => ext.to_u32(data),
            Extractor::Dynamic(ext) => ext.to_u32(data),
        }
    }

    /// Parses a `u64` from the data slice.
    #[inline]
    pub fn to_u64(&self, data: &[u8]) -> Result<u64, ParseErr> {
        match self {
            Extractor::Fixed(ext) => ext.to_u64(data),
            Extractor::Dynamic(ext) => ext.to_u64(data),
        }
    }

    /// Parses a `u128` from the data slice.
    #[inline]
    pub fn to_u128(&self, data: &[u8]) -> Result<u128, ParseErr> {
        match self {
            Extractor::Fixed(ext) => ext.to_u128(data),
            Extractor::Dynamic(ext) => ext.to_u128(data),
        }
    }

    /// Parses an `i8` from the data slice.
    #[inline]
    pub fn to_i8(&self, data: &[u8]) -> Result<i8, ParseErr> {
        match self {
            Extractor::Fixed(ext) => ext.to_i8(data),
            Extractor::Dynamic(ext) => ext.to_i8(data),
        }
    }

    /// Parses an `i16` from the data slice.
    #[inline]
    pub fn to_i16(&self, data: &[u8]) -> Result<i16, ParseErr> {
        match self {
            Extractor::Fixed(ext) => ext.to_i16(data),
            Extractor::Dynamic(ext) => ext.to_i16(data),
        }
    }

    /// Parses an `i32` from the data slice.
    #[inline]
    pub fn to_i32(&self, data: &[u8]) -> Result<i32, ParseErr> {
        match self {
            Extractor::Fixed(ext) => ext.to_i32(data),
            Extractor::Dynamic(ext) => ext.to_i32(data),
        }
    }

    /// Parses an `i64` from the data slice.
    #[inline]
    pub fn to_i64(&self, data: &[u8]) -> Result<i64, ParseErr> {
        match self {
            Extractor::Fixed(ext) => ext.to_i64(data),
            Extractor::Dynamic(ext) => ext.to_i64(data),
        }
    }

    /// Parses an `i128` from the data slice.
    #[inline]
    pub fn to_i128(&self, data: &[u8]) -> Result<i128, ParseErr> {
        match self {
            Extractor::Fixed(ext) => ext.to_i128(data),
            Extractor::Dynamic(ext) => ext.to_i128(data),
        }
    }

    /// [`to_i64`](Self::to_i64) with the decimal point's presence verified
    /// first — see
    /// [`FixedExtractor::to_i64_checked`](fixed_extractor::FixedExtractor::to_i64_checked).
    ///
    /// A dynamic extractor reads the point out of the text it was given, so
    /// there is nothing to verify and this is just `to_i64`.
    #[inline]
    pub fn to_i64_checked(&self, data: &[u8]) -> Result<i64, ParseErr> {
        match self {
            Extractor::Fixed(ext) => ext.to_i64_checked(data),
            Extractor::Dynamic(ext) => ext.to_i64(data),
        }
    }

    /// Parses an `f32` from the data slice.
    #[inline]
    pub fn to_f32(&self, data: &[u8]) -> Result<f32, ParseErr> {
        match self {
            Extractor::Fixed(ext) => ext.to_f32(data),
            Extractor::Dynamic(ext) => ext.to_f32(data),
        }
    }

    /// Parses an `f64` from the data slice.
    #[inline]
    pub fn to_f64(&self, data: &[u8]) -> Result<f64, ParseErr> {
        match self {
            Extractor::Fixed(ext) => ext.to_f64(data),
            Extractor::Dynamic(ext) => ext.to_f64(data),
        }
    }

    /// Returns [`Scale`] for this extractor's decimal places, or `None` if > 8.
    #[inline]
    pub fn scale(&self) -> Option<Scale> {
        match self {
            Extractor::Fixed(ext) => Scale::from_decimals(ext.config.fraction_size),
            Extractor::Dynamic(ext) => ext.scale(),
        }
    }

    /// Returns the number of decimal places.
    #[inline]
    pub fn decimals(&self) -> usize {
        match self {
            Extractor::Fixed(ext) => ext.config.fraction_size,
            Extractor::Dynamic(ext) => ext.target_decimals,
        }
    }
}

impl From<Config> for Extractor {
    fn from(config: Config) -> Self {
        Self::Fixed(FixedExtractor::new(config))
    }
}
