//! Fixed-width ASCII field readers.
//!
//! KRX messages are fixed-width ASCII with no separators, so every reader here
//! takes the exact slice the layout table gives it plus its offset (for error
//! reporting) and nothing else. The offsets come from
//! `documents/krx/layouts.md`, which is generated — the spec's own offset
//! column is an **end** offset and counting it by hand shifts every field by
//! one (`CLAUDE.md`).
//!
//! ## Blank is a value
//!
//! A field of all spaces means **"not measurable"**, which is not the same as
//! zero. 정보분배일련번호 arrives blank on whole channels, and reading it as 0
//! makes every message look like a sequence gap. Every reader here therefore
//! returns `Option`, and a decoder that wants to treat blank as zero has to say
//! so out loud.
//!
//! Note that the converse also happens: some fields carry `000000.00` rather
//! than blanks when the value does not apply (dynamic price limits on
//! instruments outside the regime). "Zero" and "not applicable" are not
//! distinguishable in the raw message, which is why that conclusion rides in a
//! wire flag instead of the value.

use crate::error::KrxError;
use jeed_wire::{ISIN_LEN, Isin, Scale};

/// Nanoseconds in one day — a time-of-day reading is always below this.
pub const NS_PER_DAY: u64 = 24 * 60 * 60 * 1_000_000_000;

/// A fixed-width ASCII decimal, exactly as the message spells it.
///
/// The number of decimals is read off the message rather than looked up,
/// because the layout varies by *instrument*, not by message: derivative
/// real-time prices are nine bytes either way, but KOSPI200 futures spell them
/// `[sign][5].[2]` while single-stock futures spell them `[sign][8]` with no
/// point at all — and three-month risk-free-rate futures use `[sign][4].[3]`
/// while sharing the product group `06F` with instruments that use `[5].[2]`.
/// Scanning for the point is the only rule that holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decimal {
    /// All digits as one integer, sign applied, point ignored.
    pub value: i64,

    /// Digits after the decimal point.
    pub decimals: u8,
}

impl Decimal {
    /// The [`Scale`] this field is expressed in, for the record header.
    ///
    /// `None` past eight places, which no KRX field reaches.
    #[inline]
    pub const fn scale(&self) -> Option<Scale> {
        Scale::from_decimals(self.decimals as usize)
    }
}

/// `true` when every byte is a space.
#[inline]
pub const fn is_blank(field: &[u8]) -> bool {
    let mut i = 0;
    while i < field.len() {
        if field[i] != b' ' {
            return false;
        }
        i += 1;
    }
    true
}

/// Reads a signed fixed-width decimal: `[sign][digits and at most one point]`.
///
/// The sign byte is `'0'` (or `'+'`) for positive and `'-'` for negative; KRX
/// only ever sends a negative in a spread quote. Equity prices spell an unused
/// byte after the sign, which is a `'0'` and so reads as a leading digit.
///
/// `at` is the field's offset in the message and only reaches error values.
pub const fn decimal(field: &[u8], at: usize) -> Result<Option<Decimal>, KrxError> {
    if field.is_empty() {
        return Err(KrxError::TooShort { need: 1, got: 0 });
    }
    if is_blank(field) {
        return Ok(None);
    }

    let negative = match field[0] {
        b'0' | b'+' => false,
        b'-' => true,
        found => return Err(KrxError::Sign { at, found }),
    };

    let mut value: i64 = 0;
    let mut decimals: i32 = -1; // -1 until a point is seen
    let mut i = 1;
    while i < field.len() {
        let b = field[i];
        if b.is_ascii_digit() {
            value = match value.checked_mul(10) {
                Some(v) => v,
                None => return Err(KrxError::Overflow { at: at + i }),
            };
            value = match value.checked_add((b - b'0') as i64) {
                Some(v) => v,
                None => return Err(KrxError::Overflow { at: at + i }),
            };
            if decimals >= 0 {
                decimals += 1;
            }
        } else if b == b'.' {
            if decimals >= 0 {
                return Err(KrxError::DecimalPoint { at: at + i });
            }
            decimals = 0;
        } else {
            return Err(KrxError::Digit { at: at + i, found: b });
        }
        i += 1;
    }

    let decimals = if decimals < 0 { 0 } else { decimals as u8 };
    Ok(Some(Decimal { value: if negative { -value } else { value }, decimals }))
}

/// Reads an unsigned fixed-width integer — quantities, counts, sequence
/// numbers. No sign byte and no decimal point.
pub const fn uint(field: &[u8], at: usize) -> Result<Option<u64>, KrxError> {
    if is_blank(field) {
        return Ok(None);
    }
    let mut value: u64 = 0;
    let mut i = 0;
    while i < field.len() {
        let b = field[i];
        if !b.is_ascii_digit() {
            return Err(KrxError::Digit { at: at + i, found: b });
        }
        value = match value.checked_mul(10) {
            Some(v) => v,
            None => return Err(KrxError::Overflow { at: at + i }),
        };
        value = match value.checked_add((b - b'0') as u64) {
            Some(v) => v,
            None => return Err(KrxError::Overflow { at: at + i }),
        };
        i += 1;
    }
    Ok(Some(value))
}

/// Reads `HHMMSS` plus a sub-second part as nanoseconds since midnight.
///
/// Twelve bytes is `HHMMSSuuuuuu` (microseconds, the real-time feeds); nine is
/// `HHMMSSmmm` (milliseconds, e.g. 가격확대시각 on `V1`).
///
/// **The result has no date**, so it wraps at midnight. Assembling an absolute
/// timestamp is the receive loop's job, not the field reader's — and it stays
/// inside the handler (`documents/feed_handler.md` §6).
pub const fn time_of_day_ns(field: &[u8], at: usize) -> Result<Option<u64>, KrxError> {
    let sub_digits = match field.len() {
        12 => 6,
        9 => 3,
        len => return Err(KrxError::TimeWidth { len }),
    };
    if is_blank(field) {
        return Ok(None);
    }

    let hh = match uint(field.split_at(2).0, at) {
        Ok(Some(v)) => v,
        Ok(None) => return Ok(None),
        Err(e) => return Err(e),
    };
    let (_, rest) = field.split_at(2);
    let mm = match uint(rest.split_at(2).0, at + 2) {
        Ok(Some(v)) => v,
        Ok(None) => return Ok(None),
        Err(e) => return Err(e),
    };
    let (_, rest) = rest.split_at(2);
    let ss = match uint(rest.split_at(2).0, at + 4) {
        Ok(Some(v)) => v,
        Ok(None) => return Ok(None),
        Err(e) => return Err(e),
    };
    let (_, sub) = rest.split_at(2);
    let sub = match uint(sub, at + 6) {
        Ok(Some(v)) => v,
        Ok(None) => return Ok(None),
        Err(e) => return Err(e),
    };

    // 24:00:00 is not a clock reading, and neither is 09:61.
    if hh > 23 || mm > 59 || ss > 59 {
        return Err(KrxError::TimeRange { at });
    }

    let sub_ns = if sub_digits == 6 { sub * 1_000 } else { sub * 1_000_000 };
    Ok(Some(((hh * 3600 + mm * 60 + ss) * 1_000_000_000) + sub_ns))
}

/// Copies a twelve-byte ISIN out of the message.
///
/// Raw bytes, deliberately: the wire carries `(venue, isin)` because interned
/// identifiers are process-local and cannot cross the segment
/// (`documents/feed_handler.md` §5).
#[inline]
pub const fn isin(field: &[u8]) -> Result<Isin, KrxError> {
    if field.len() != ISIN_LEN {
        return Err(KrxError::TooShort { need: ISIN_LEN, got: field.len() });
    }
    let mut out = [0u8; ISIN_LEN];
    let mut i = 0;
    while i < ISIN_LEN {
        out[i] = field[i];
        i += 1;
    }
    Ok(out)
}
