//! Decode errors.
//!
//! Every variant is `Copy` and names the JSON key that stopped making sense,
//! because the only useful thing to do with a bad frame is log it next to the
//! raw bytes and drop it. Where `jeed_krx::KrxError` names a byte offset, this
//! names a key: a fixed-width message has an offset and a JSON object does
//! not.

use core::fmt;
use jeed_convert::ParseErr;
use jeed_wire::WireError;

/// A crypto market-data frame could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError {
    /// The frame was empty, or held no JSON object.
    Empty,

    /// A decimal-string field did not parse on the instrument's scale.
    ///
    /// [`ParseErr::Precision`] here means the *scale is wrong*, not the
    /// message: the venue sent a digit the configured scale cannot keep, so
    /// the instrument's `exchangeInfo` precision has changed or was mis-typed.
    /// Dropping the frame is right — publishing a truncated price would be a
    /// plausible-looking wrong number (`jeed_convert::DynamicExtractor`).
    Field {
        /// JSON key, as the venue spells it.
        key: &'static str,

        /// What the parser objected to.
        err: ParseErr,
    },

    /// A field this message cannot be decoded without was absent.
    Missing {
        /// JSON key, as the venue spells it.
        key: &'static str,
    },

    /// A routing field held a value this build does not know.
    ///
    /// OKX's `action` and Bybit's `type` decide whether a frame is a whole
    /// book or a diff, and there is no safe default: reading a diff as a
    /// snapshot throws the book away, and reading a snapshot as a diff doubles
    /// every level. A third value means the venue changed its protocol, and
    /// the only correct response is to stop decoding it.
    Unexpected {
        /// JSON key whose value was not recognised.
        key: &'static str,
    },

    /// The frame's `s` names a different instrument than the decoder is
    /// pinned to.
    ///
    /// One decoder is one subscription, so this is a wiring mistake — a
    /// stream plugged into the wrong decoder — and it must be loud. Left
    /// unchecked it would publish one instrument's prices under another's
    /// symbol, which is the quietest possible way to be wrong.
    SymbolMismatch,

    /// More changed levels than a [`SnapshotDeltaPayload`] holds.
    ///
    /// Deliberately fatal to the frame rather than truncating: see
    /// [`SnapshotDeltaPayload`]. Nothing is published, and the consumer meets
    /// the resulting hole in the update-id chain and resynchronises.
    ///
    /// [`SnapshotDeltaPayload`]: jeed_wire::SnapshotDeltaPayload
    DeltaOverflow {
        /// Levels the payload holds, across both sides.
        max: usize,
    },

    /// A symbol or scale the wire cannot carry. Raised when an
    /// [`Instrument`](crate::Instrument) is built, never mid-frame.
    Instrument(InstrumentError),

    /// The decoded values do not fit the wire record.
    Wire(WireError),
}

/// Why an [`Instrument`](crate::Instrument) could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstrumentError {
    /// Symbol is empty, longer than [`SYMBOL_LEN`], or holds a `NUL`.
    ///
    /// [`SYMBOL_LEN`]: jeed_wire::SYMBOL_LEN
    Symbol,

    /// Decimal count exceeds what [`Scale`](jeed_wire::Scale) encodes (eight).
    Scale {
        /// Decimals asked for.
        decimals: usize,
    },
}

impl From<WireError> for CryptoError {
    #[inline]
    fn from(e: WireError) -> Self {
        Self::Wire(e)
    }
}

impl From<InstrumentError> for CryptoError {
    #[inline]
    fn from(e: InstrumentError) -> Self {
        Self::Instrument(e)
    }
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "frame carries no JSON object"),
            Self::Field { key, err } => write!(f, "field {key:?}: {err}"),
            Self::Missing { key } => write!(f, "field {key:?} is absent"),
            Self::Unexpected { key } => write!(f, "field {key:?} holds an unrecognised value"),
            Self::SymbolMismatch => write!(f, "frame is for another instrument"),
            Self::DeltaOverflow { max } => {
                write!(f, "delta carries more changed levels than the {max} the record holds")
            }
            Self::Instrument(e) => write!(f, "{e}"),
            Self::Wire(e) => write!(f, "{e}"),
        }
    }
}

impl fmt::Display for InstrumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Symbol => write!(f, "symbol does not fit the wire's symbol field"),
            Self::Scale { decimals } => write!(f, "{decimals} decimals exceeds the 8 a scale encodes"),
        }
    }
}

impl core::error::Error for CryptoError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Wire(e) => Some(e),
            Self::Instrument(e) => Some(e),
            _ => None,
        }
    }
}

impl core::error::Error for InstrumentError {}
