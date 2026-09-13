//! Decode errors.
//!
//! Every variant is `Copy` and names the byte offset where the message stopped
//! making sense, because the only useful thing to do with a bad message is to
//! log it next to the raw bytes and drop it.

use crate::trcode::TrCode;
use core::fmt;
use jeed_wire::WireError;

/// A KRX message could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KrxError {
    /// Datagram is shorter than the fixed part it must contain.
    TooShort {
        /// Bytes needed.
        need: usize,

        /// Bytes present.
        got: usize,
    },

    /// Datagram length does not match the length this trcode is defined with.
    ///
    /// KRX messages are fixed length per interface, so this is checked
    /// **before** any field is read.
    Length {
        /// Length the interface defines.
        expected: usize,

        /// Length received.
        actual: usize,
    },

    /// Last byte is not the end keyword (`0xFF`).
    EndKeyword {
        /// Byte found in its place.
        found: u8,
    },

    /// The message's trcode is not one this build decodes.
    UnknownTrCode {
        /// The five raw bytes.
        code: TrCode,
    },

    /// A numeric field holds something that is not a digit.
    Digit {
        /// Offset within the message.
        at: usize,

        /// Byte found.
        found: u8,
    },

    /// A signed field's sign byte is neither `'0'`, `'+'` nor `'-'`.
    Sign {
        /// Offset within the message.
        at: usize,

        /// Byte found.
        found: u8,
    },

    /// A decimal field carries more than one decimal point.
    DecimalPoint {
        /// Offset within the message.
        at: usize,
    },

    /// A numeric field does not fit the wire's integer width.
    Overflow {
        /// Offset within the message.
        at: usize,
    },

    /// A time-of-day field is not `HHMMSS` plus a sub-second part of a width
    /// this decoder knows.
    TimeWidth {
        /// Field width found.
        len: usize,
    },

    /// A time-of-day field is syntactically fine but not a real clock reading.
    TimeRange {
        /// Offset within the message.
        at: usize,
    },

    /// The decoded values do not fit the wire record.
    Wire(WireError),
}

impl From<WireError> for KrxError {
    #[inline]
    fn from(e: WireError) -> Self {
        Self::Wire(e)
    }
}

impl fmt::Display for KrxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { need, got } => {
                write!(f, "datagram is {got} bytes, needs at least {need}")
            }
            Self::Length { expected, actual } => {
                write!(f, "message length {actual}, interface defines {expected}")
            }
            Self::EndKeyword { found } => write!(f, "end keyword is {found:#04x}, not 0xff"),
            Self::UnknownTrCode { code } => write!(f, "trcode {code} is not decoded"),
            Self::Digit { at, found } => {
                write!(f, "byte {at}: {:?} is not a digit", *found as char)
            }
            Self::Sign { at, found } => {
                write!(f, "byte {at}: {:?} is not a sign", *found as char)
            }
            Self::DecimalPoint { at } => write!(f, "byte {at}: second decimal point"),
            Self::Overflow { at } => write!(f, "byte {at}: number too wide for the wire"),
            Self::TimeWidth { len } => write!(f, "time field is {len} bytes"),
            Self::TimeRange { at } => write!(f, "byte {at}: not a clock reading"),
            Self::Wire(e) => write!(f, "{e}"),
        }
    }
}

impl core::error::Error for KrxError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Wire(e) => Some(e),
            _ => None,
        }
    }
}
