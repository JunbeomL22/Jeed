//! Fixed-width ASCII field readers.
//!
//! KRX messages are fixed-width ASCII with no separators, so every reader here
//! takes the exact slice the layout table gives it plus its offset (for error
//! reporting) and nothing else. The offsets come from
//! `documents/krx/layouts.md`, which is generated — the spec's own offset
//! column is an **end** offset and counting it by hand shifts every field by
//! one (`CLAUDE.md`).
//!
//! The parsing itself is [`jeed_convert`]: fields are loaded a machine word at
//! a time and folded with bitwise operations rather than walked byte by byte,
//! and a price's scale is a property of the reader that was chosen
//! ([`crate::extract`]) rather than something discovered per message and
//! carried alongside the value.
//!
//! ## Blank is a value
//!
//! A field of all spaces means **"not measurable"**, which is not the same as
//! zero. 정보분배일련번호 arrives blank on whole channels, and reading it as 0
//! makes every message look like a sequence gap. Every reader here therefore
//! checks for blank first and returns `Option`, and a decoder that wants to
//! treat blank as zero has to say so out loud.
//!
//! Blank is checked rather than inferred from a parse failure. Swallowing the
//! parser's error would fold "the exchange sent spaces" together with "these
//! bytes are corrupt", and those call for opposite reactions: the first is
//! normal, the second must drop the message.
//!
//! Note that the converse also happens: some fields carry `000000.00` rather
//! than blanks when the value does not apply (dynamic price limits on
//! instruments outside the regime). "Zero" and "not applicable" are not
//! distinguishable in the raw message, which is why that conclusion rides in a
//! wire flag instead of the value.

use crate::error::KrxError;
use jeed_convert::{Biscuit, Extractor};
use jeed_wire::{ISIN_LEN, Isin};

/// Nanoseconds in one day — a time-of-day reading is always below this.
pub const NS_PER_DAY: u64 = 24 * 60 * 60 * 1_000_000_000;

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

/// Reads a signed fixed-width price with the reader its instrument calls for.
///
/// The sign byte is `'0'` (or `'+'`) for positive and `'-'` for negative; KRX
/// only ever sends a negative in a spread quote. Equity prices spell an unused
/// byte after the sign, which is a `'0'` and so reads as a leading digit.
///
/// The returned integer is in the reader's own scale — `93705` for `000937.05`
/// read as `[부호][5].[2]`. That scale goes on the record header once, not on
/// every value.
///
/// `at` is the field's offset in the message and only reaches error values.
#[inline]
pub fn price(reader: &Extractor, field: &[u8], at: usize) -> Result<Option<i64>, KrxError> {
    if is_blank(field) {
        return Ok(None);
    }
    reader.to_i64_checked(field).map(Some).map_err(|err| KrxError::Field { at, err })
}

/// Reads an unsigned fixed-width integer — quantities, counts, sequence
/// numbers. No sign byte and no decimal point, so no reader is needed.
#[inline]
pub fn uint(field: &[u8], at: usize) -> Result<Option<u64>, KrxError> {
    if is_blank(field) {
        return Ok(None);
    }
    u64::parse_decimal(field).map(Some).map_err(|err| KrxError::Field { at, err })
}

/// Reads `HHMMSS` plus a sub-second part as nanoseconds since midnight.
///
/// Twelve bytes is `HHMMSSuuuuuu` (microseconds, the real-time feeds); nine is
/// `HHMMSSmmm` (milliseconds, e.g. 가격확대시각 on `V1`).
///
/// **The result has no date**, so it wraps at midnight. Assembling an absolute
/// timestamp is [`crate::clock`]'s job, not the field reader's — and it stays
/// inside the handler (`documents/feed_handler.md` §6).
pub fn time_of_day_ns(field: &[u8], at: usize) -> Result<Option<u64>, KrxError> {
    let sub_digits = match field.len() {
        12 => 6usize,
        9 => 3,
        _ => return Err(KrxError::Field { at, err: jeed_convert::ParseErr::InvalidLength }),
    };
    if is_blank(field) {
        return Ok(None);
    }

    let (hhmmss, sub) = field.split_at(6);

    // Range-check the digits before folding, not after. The fold multiplies the
    // minutes field by 60 and the hours field by 3600 whatever they say, so
    // `096100` comes out of it as a perfectly ordinary second count — the
    // nonsense is only visible while the digits are still digits. Three
    // comparisons on the tens places is the whole check: minutes and seconds
    // cannot start above '5', and hours cannot exceed 23.
    if hhmmss[0] > b'2'
        || (hhmmss[0] == b'2' && hhmmss[1] > b'3')
        || hhmmss[2] > b'5'
        || hhmmss[4] > b'5'
    {
        return Err(KrxError::TimeRange { at });
    }

    let seconds = jeed_convert::decimal_core::hhmmss::parse_hhmmss_to_seconds(hhmmss)
        .map_err(|err| KrxError::Field { at, err })?;

    let sub = match u64::parse_decimal(sub) {
        Ok(v) => v,
        Err(err) => return Err(KrxError::Field { at: at + 6, err }),
    };
    let sub_ns = if sub_digits == 6 { sub * 1_000 } else { sub * 1_000_000 };
    Ok(Some(seconds * 1_000_000_000 + sub_ns))
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
