//! FIX parse errors — every variant is `Copy` and carries no heap data.

use std::fmt;

/// Error raised while framing or decoding a FIX message.
///
/// A framing error ([`Self::BeginString`], [`Self::BodyLength`],
/// [`Self::CheckSum`], [`Self::Trailer`]) means the byte stream is out of
/// sync and the session must be torn down; a field error names the tag so
/// the caller can decide whether to drop the message or the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixError {
    /// The buffer ends before the message does — read more bytes.
    Incomplete,

    /// The message does not start with `8=`.
    BeginString,

    /// The `9=` field is missing, non-numeric, or larger than
    /// [`FIX_MAX_BODY_LENGTH`](crate::FIX_MAX_BODY_LENGTH).
    BodyLength {
        /// Declared length (`u32::MAX` when unparsable).
        declared: u32,
    },

    /// The bytes after the body are not `10=ddd<SOH>`.
    Trailer,

    /// `CheckSum` disagrees with the byte sum.
    CheckSum {
        /// Sum of the framed bytes modulo 256.
        expected: u8,

        /// Value carried in `10=`.
        found: u8,
    },

    /// A field is not `tag=value<SOH>` (empty tag, missing `=`, missing `<SOH>`).
    Field {
        /// Byte offset of the offending field inside the body.
        offset: u32,
    },

    /// A required tag is absent.
    MissingTag {
        /// The tag.
        tag: u32,
    },

    /// A tag's value cannot be parsed as the expected type.
    BadValue {
        /// The tag.
        tag: u32,
    },

    /// A decimal value carries more fractional digits than the target scale.
    Precision {
        /// The tag.
        tag: u32,
    },

    /// `MsgType` is not a market-data or session message this parser handles.
    UnexpectedMsgType {
        /// First byte of `35=`.
        found: u8,
    },

    /// `NoMDEntries` declares more entries than the fixed buffer holds.
    TooManyEntries {
        /// Declared count.
        declared: u32,

        /// Buffer capacity.
        max: u32,
    },

    /// `NoMDEntries` disagrees with the number of entries actually present.
    EntryCount {
        /// Declared count.
        declared: u32,

        /// Entries found.
        found: u32,
    },

    /// A group tag appeared before the first group delimiter.
    GroupTagOutsideEntry {
        /// The tag.
        tag: u32,
    },

    /// `MDUpdateAction` is not a value this parser knows.
    ///
    /// There is deliberately no counterpart for `MDEntryType` (269): an
    /// unknown entry type is carried as
    /// [`MdEntryType::Other`](crate::market_data::MdEntryType::Other) and
    /// ignored by the venue adapter, because a venue adding a statistic entry
    /// must not break the book.
    UnknownUpdateAction {
        /// Raw byte.
        found: u8,
    },
}

impl fmt::Display for FixError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Incomplete => write!(f, "FIX message incomplete"),
            Self::BeginString => write!(f, "FIX message does not start with 8="),
            Self::BodyLength { declared } => write!(f, "FIX BodyLength invalid: {declared}"),
            Self::Trailer => write!(f, "FIX trailer is not 10=ddd<SOH>"),
            Self::CheckSum { expected, found } => {
                write!(f, "FIX CheckSum mismatch: expected {expected:03}, got {found:03}")
            }
            Self::Field { offset } => write!(f, "FIX field malformed at body offset {offset}"),
            Self::MissingTag { tag } => write!(f, "FIX tag {tag} missing"),
            Self::BadValue { tag } => write!(f, "FIX tag {tag} has an unparsable value"),
            Self::Precision { tag } => write!(f, "FIX tag {tag} exceeds the target scale"),
            Self::UnexpectedMsgType { found } => {
                write!(f, "FIX MsgType {:?} not handled", *found as char)
            }
            Self::TooManyEntries { declared, max } => {
                write!(f, "FIX NoMDEntries {declared} exceeds capacity {max}")
            }
            Self::EntryCount { declared, found } => {
                write!(f, "FIX NoMDEntries {declared} but {found} entries present")
            }
            Self::GroupTagOutsideEntry { tag } => {
                write!(f, "FIX group tag {tag} before the first entry")
            }
            Self::UnknownUpdateAction { found } => {
                write!(f, "FIX MDUpdateAction {:?} unknown", *found as char)
            }
        }
    }
}

impl std::error::Error for FixError {}
