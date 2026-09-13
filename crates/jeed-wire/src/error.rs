//! Errors raised while validating wire records.

use crate::kind::WireKind;
use crate::types::Venue;
use core::fmt;

/// Wire-record validation error.
///
/// Every variant is `Copy` and carries no heap data. A wire error means the
/// two sides of the ABI disagree, and the correct reaction is to stop reading
/// the segment — not to skip the record and carry on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    /// Byte slice length does not match the expected size.
    Length {
        /// Expected size in bytes.
        expected: usize,

        /// Actual slice length.
        actual: usize,
    },

    /// Byte slice is not aligned for an in-place reference.
    Alignment {
        /// Required alignment in bytes.
        required: usize,
    },

    /// Segment magic does not match.
    Magic {
        /// Magic value found in the segment.
        found: u64,
    },

    /// Segment format version differs from this binary's.
    FormatVersion {
        /// Version compiled into this binary.
        expected: u32,

        /// Version written by the producer.
        found: u32,
    },

    /// Segment record size differs from this binary's.
    RecordSize {
        /// Record size compiled into this binary.
        expected: u32,

        /// Record size written by the producer.
        found: u32,
    },

    /// Segment capacity is zero.
    Capacity,

    /// Record kind byte is not a known [`WireKind`].
    Kind {
        /// Raw kind byte.
        found: u8,
    },

    /// A typed payload accessor was used on a record of another kind.
    KindMismatch {
        /// Kind the accessor expects.
        expected: WireKind,

        /// Kind the record actually carries.
        found: WireKind,
    },

    /// Venue byte is not a known [`Venue`].
    Venue {
        /// Raw venue byte.
        found: u8,
    },

    /// Scale byte is not a known [`Scale`](crate::types::Scale).
    Scale {
        /// Raw scale byte.
        found: u8,
    },

    /// Depth exceeds [`WIRE_MAX_DEPTH`](crate::WIRE_MAX_DEPTH).
    Depth {
        /// Depth carried by the record.
        found: u8,

        /// Maximum depth supported by the layout.
        max: u8,
    },

    /// Trade kind byte is not a known encoding.
    TradeKind {
        /// Raw trade kind byte.
        found: u8,
    },

    /// Level extension kind byte is not a known encoding.
    LevelExtKind {
        /// Raw level extension kind byte.
        found: u8,
    },

    /// Quote extension kind byte is not a known encoding.
    QuoteExtKind {
        /// Raw quote extension kind byte.
        found: u8,
    },

    /// Expansion direction byte is not a known encoding.
    ExpansionDirection {
        /// Raw direction byte.
        found: u8,
    },
}

impl WireError {
    /// Venue error for a decoded venue, for callers that already have one.
    #[inline]
    pub const fn venue(v: Venue) -> Self {
        Self::Venue { found: v.as_u8() }
    }
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length { expected, actual } => {
                write!(f, "wire length mismatch: expected {expected}, got {actual}")
            }
            Self::Alignment { required } => write!(f, "wire slice not aligned to {required}"),
            Self::Magic { found } => write!(f, "wire segment magic mismatch: {found:#x}"),
            Self::FormatVersion { expected, found } => {
                write!(f, "wire format version mismatch: expected {expected}, got {found}")
            }
            Self::RecordSize { expected, found } => {
                write!(f, "wire record size mismatch: expected {expected}, got {found}")
            }
            Self::Capacity => write!(f, "wire segment capacity is zero"),
            Self::Kind { found } => write!(f, "unknown wire record kind {found}"),
            Self::KindMismatch { expected, found } => {
                write!(f, "wire record kind mismatch: expected {expected:?}, got {found:?}")
            }
            Self::Venue { found } => write!(f, "unknown wire venue {found}"),
            Self::Scale { found } => write!(f, "unknown wire scale {found}"),
            Self::Depth { found, max } => write!(f, "wire depth {found} exceeds {max}"),
            Self::TradeKind { found } => write!(f, "unknown wire trade kind {found}"),
            Self::LevelExtKind { found } => write!(f, "unknown wire level extension {found}"),
            Self::QuoteExtKind { found } => write!(f, "unknown wire quote extension {found}"),
            Self::ExpansionDirection { found } => {
                write!(f, "unknown wire expansion direction {found}")
            }
        }
    }
}

impl core::error::Error for WireError {}
