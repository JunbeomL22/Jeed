//! Errors from creating, attaching to, and reading a segment.

use core::fmt;
use jeed_wire::WireError;

/// Failure while mapping or attaching a shared-memory segment.
///
/// Every variant is `Copy`: an error here is a startup or configuration fault,
/// not something the hot loop allocates for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShmError {
    /// Segment name was empty.
    NameEmpty,

    /// Segment name is longer than [`MAX_NAME_LEN`](crate::MAX_NAME_LEN).
    NameTooLong {
        /// Length supplied.
        len: usize,
    },

    /// Segment name contains a byte that is not allowed in a Win32 object
    /// name (a path separator, or a non-printable / non-ASCII byte).
    NameChar {
        /// Offending byte.
        byte: u8,
    },

    /// Capacity is zero or not a power of two.
    ///
    /// The power-of-two requirement is not decoration: it turns the slot index
    /// into a mask instead of a 64-bit division in the hot path.
    Capacity {
        /// Capacity requested.
        requested: u64,
    },

    /// A Win32 call failed.
    Os {
        /// Name of the call that failed.
        call: &'static str,

        /// `GetLastError()` at the point of failure.
        code: u32,
    },

    /// The mapped section is smaller than the layout it claims to hold.
    ///
    /// On the producer side this means an existing section of the same name
    /// was created with a smaller capacity; on the consumer side it means the
    /// header is lying, and the segment must be refused rather than read.
    SegmentTooSmall {
        /// Bytes the layout needs.
        needed: usize,

        /// Bytes actually mapped.
        mapped: usize,
    },

    /// The producer's wire layout disagrees with this binary's
    /// ([`SegmentHeader::check`](jeed_wire::SegmentHeader::check)).
    Wire(WireError),
}

impl From<WireError> for ShmError {
    #[inline]
    fn from(e: WireError) -> Self {
        Self::Wire(e)
    }
}

impl fmt::Display for ShmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NameEmpty => write!(f, "segment name is empty"),
            Self::NameTooLong { len } => {
                write!(f, "segment name is {len} bytes, max {}", crate::MAX_NAME_LEN)
            }
            Self::NameChar { byte } => write!(f, "segment name contains byte {byte:#04x}"),
            Self::Capacity { requested } => {
                write!(f, "capacity {requested} is not a non-zero power of two")
            }
            Self::Os { call, code } => write!(f, "{call} failed with GetLastError {code}"),
            Self::SegmentTooSmall { needed, mapped } => {
                write!(f, "segment needs {needed} bytes, {mapped} mapped")
            }
            Self::Wire(e) => write!(f, "{e}"),
        }
    }
}

impl core::error::Error for ShmError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Wire(e) => Some(e),
            _ => None,
        }
    }
}
