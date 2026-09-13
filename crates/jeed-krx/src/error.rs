//! Decode errors.
//!
//! Every variant is `Copy` and names the byte offset where the message stopped
//! making sense, because the only useful thing to do with a bad message is to
//! log it next to the raw bytes and drop it.

use crate::trcode::TrCode;
use core::fmt;
use jeed_convert::ParseErr;
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

    /// A fixed-width numeric field did not parse.
    ///
    /// This is where a mis-selected price shape lands. A derivative price is
    /// nine bytes in three shapes, and each expects the decimal point at a
    /// different index; reading one as another puts a `'.'` where a digit
    /// belongs and fails here rather than silently shifting the value by a
    /// factor of ten (`extract`).
    Field {
        /// Offset within the message.
        at: usize,

        /// What the parser objected to.
        err: ParseErr,
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
            Self::Field { at, err } => write!(f, "byte {at}: {err}"),
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
