//! The five-byte message type code.
//!
//! ```text
//! B 6 0 1 F
//! └─┬─┘ └┬─┘
//!   │    └── 정보구분+시장구분 3B — the product group (01F = KOSPI200 futures)
//!   └─────── 데이터구분 2B — the data class (B6 = best quotes)
//! ```
//!
//! **The dispatch key is all five bytes.** The data class alone does not
//! determine the layout: `B6` spans eight interfaces from 324 to 1387 bytes
//! (derivative 5-deep 324, derivative 10-deep 554, equity 590, bond 462,
//! small-lot bond 882, REPO 1387, gold 795, emission 325). Only the product group
//! settles which.

use crate::error::KrxError;
use core::fmt;

/// Bytes of a trcode.
pub const TRCODE_LEN: usize = 5;

/// Offset of the trcode in a message — it is the first field.
pub const TRCODE_OFFSET: usize = 0;

/// A five-byte KRX message type code.
///
/// Packs into a `u64` ([`as_u64`](Self::as_u64)) so an allow-set is integer
/// comparisons rather than slice comparisons in the receive loop.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct TrCode([u8; TRCODE_LEN]);

impl TrCode {
    /// Wraps five raw bytes. Not validated: an unknown code is a dispatch
    /// miss, not a parse error.
    #[inline]
    pub const fn new(bytes: [u8; TRCODE_LEN]) -> Self {
        Self(bytes)
    }

    /// Reads the trcode off the front of a datagram.
    #[inline]
    pub const fn from_message(payload: &[u8]) -> Result<Self, KrxError> {
        if payload.len() < TRCODE_LEN {
            return Err(KrxError::TooShort { need: TRCODE_LEN, got: payload.len() });
        }
        Ok(Self([payload[0], payload[1], payload[2], payload[3], payload[4]]))
    }

    /// The raw bytes.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; TRCODE_LEN] {
        &self.0
    }

    /// 데이터구분 — the first two bytes (`B6`, `G7`, `A3`, `V1`, `Q2`, `M4`).
    #[inline]
    pub const fn data_class(&self) -> [u8; 2] {
        [self.0[0], self.0[1]]
    }

    /// 정보구분+시장구분 — the last three bytes (`01F`, `03F`, `04S`).
    ///
    /// This is what the multicast group is carved by, so it is also what
    /// decides which sockets are worth opening.
    #[inline]
    pub const fn product_group(&self) -> [u8; 3] {
        [self.0[2], self.0[3], self.0[4]]
    }

    /// `true` for a derivative product group (trailing `F`).
    #[inline]
    pub const fn is_derivative(&self) -> bool {
        self.0[4] == b'F'
    }

    /// Little-endian packing into the low five bytes of a `u64`.
    #[inline]
    pub const fn as_u64(self) -> u64 {
        (self.0[0] as u64)
            | (self.0[1] as u64) << 8
            | (self.0[2] as u64) << 16
            | (self.0[3] as u64) << 24
            | (self.0[4] as u64) << 32
    }

    /// The inverse of [`as_u64`](Self::as_u64). Bytes above the fifth are
    /// ignored.
    #[inline]
    pub const fn from_u64(key: u64) -> Self {
        let b = key.to_le_bytes();
        Self([b[0], b[1], b[2], b[3], b[4]])
    }

    /// The code as text, when every byte is printable ASCII.
    #[inline]
    pub const fn as_str(&self) -> Option<&str> {
        let mut i = 0;
        while i < TRCODE_LEN {
            if self.0[i] < 0x21 || self.0[i] > 0x7e {
                return None;
            }
            i += 1;
        }
        // SAFETY: every byte was just checked to be printable ASCII.
        Some(unsafe { core::str::from_utf8_unchecked(&self.0) })
    }
}

impl fmt::Display for TrCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.as_str() {
            Some(s) => f.write_str(s),
            None => write!(f, "{:02x?}", self.0),
        }
    }
}

impl fmt::Debug for TrCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TrCode({self})")
    }
}

impl From<[u8; TRCODE_LEN]> for TrCode {
    #[inline]
    fn from(b: [u8; TRCODE_LEN]) -> Self {
        Self(b)
    }
}
