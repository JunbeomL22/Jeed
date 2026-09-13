//! Tag-value scanning and zero-allocation value parsers.
//!
//! A FIX body is `tag=value<SOH>` repeated. [`Fields`] walks it without
//! copying; the `parse_*` functions turn a value slice into an integer, a
//! scaled decimal, or a UTC instant. Nothing here knows message semantics.

use crate::error::FixError;
use crate::SOH;
use jeed_wire::{Scale, UnixNano};

/// One `tag=value` pair borrowed from the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field<'a> {
    /// Numeric tag.
    pub tag: u32,

    /// Value bytes, without the trailing `<SOH>`.
    pub value: &'a [u8],
}

impl<'a> Field<'a> {
    /// First byte of the value, for single-character enumerations.
    #[inline]
    pub fn first_byte(&self) -> Result<u8, FixError> {
        match self.value {
            [b] => Ok(*b),
            _ => Err(FixError::BadValue { tag: self.tag }),
        }
    }

    /// Value as an unsigned integer.
    #[inline]
    pub fn as_u64(&self) -> Result<u64, FixError> {
        parse_u64(self.value).ok_or(FixError::BadValue { tag: self.tag })
    }

    /// Value as an unsigned 32-bit integer.
    #[inline]
    pub fn as_u32(&self) -> Result<u32, FixError> {
        let v = self.as_u64()?;
        u32::try_from(v).map_err(|_| FixError::BadValue { tag: self.tag })
    }

    /// Value as a signed decimal scaled to `scale` (exact — extra fractional
    /// digits are an error, not a rounding).
    #[inline]
    pub fn as_scaled(&self, scale: Scale) -> Result<i64, FixError> {
        match parse_scaled_decimal(self.value, scale) {
            Ok(v) => Ok(v),
            Err(DecimalError::Precision) => Err(FixError::Precision { tag: self.tag }),
            Err(DecimalError::Malformed) => Err(FixError::BadValue { tag: self.tag }),
        }
    }

    /// Value as an unsigned decimal scaled to `scale`.
    #[inline]
    pub fn as_scaled_unsigned(&self, scale: Scale) -> Result<u64, FixError> {
        let v = self.as_scaled(scale)?;
        u64::try_from(v).map_err(|_| FixError::BadValue { tag: self.tag })
    }

    /// Value as a `UTCTimestamp` (`YYYYMMDD-HH:MM:SS[.fff…]`).
    #[inline]
    pub fn as_utc_timestamp(&self) -> Result<UnixNano, FixError> {
        parse_utc_timestamp(self.value).ok_or(FixError::BadValue { tag: self.tag })
    }

    /// Value as a `UTCDateOnly` (`YYYYMMDD`), returned as ns at midnight.
    #[inline]
    pub fn as_utc_date(&self) -> Result<UnixNano, FixError> {
        parse_utc_date(self.value).ok_or(FixError::BadValue { tag: self.tag })
    }

    /// Value as a `UTCTimeOnly` (`HH:MM:SS[.fff…]`), returned as ns since midnight.
    #[inline]
    pub fn as_utc_time(&self) -> Result<UnixNano, FixError> {
        parse_utc_time(self.value).ok_or(FixError::BadValue { tag: self.tag })
    }

    /// `true` when the value is the single byte `Y`.
    #[inline]
    pub fn is_yes(&self) -> bool {
        self.value == b"Y"
    }
}

/// Iterator over the fields of a body slice.
///
/// Yields `Err(FixError::Field)` once for a malformed field and then ends.
#[derive(Debug, Clone)]
pub struct Fields<'a> {
    body: &'a [u8],

    pos: usize,
}

impl<'a> Fields<'a> {
    /// Starts scanning `body` at its first byte.
    #[inline]
    pub const fn new(body: &'a [u8]) -> Self {
        Self { body, pos: 0 }
    }

    /// Byte offset of the next unread field.
    #[inline]
    pub const fn offset(&self) -> usize {
        self.pos
    }

    /// Unread remainder of the body.
    #[inline]
    pub fn rest(&self) -> &'a [u8] {
        &self.body[self.pos.min(self.body.len())..]
    }
}

impl<'a> Iterator for Fields<'a> {
    type Item = Result<Field<'a>, FixError>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let rest = self.rest();
        if rest.is_empty() {
            return None;
        }
        let start = self.pos;
        match split_field(rest) {
            Some((field, consumed)) => {
                self.pos += consumed;
                Some(Ok(field))
            }
            None => {
                self.pos = self.body.len();
                Some(Err(FixError::Field { offset: start as u32 }))
            }
        }
    }
}

/// Splits the first `tag=value<SOH>` off `bytes`; returns the field and the
/// number of bytes consumed. `None` when the head is not a complete field.
#[inline]
pub fn split_field(bytes: &[u8]) -> Option<(Field<'_>, usize)> {
    let eq = bytes.iter().position(|&b| b == b'=')?;
    if eq == 0 || eq > 10 {
        return None;
    }
    let tag = parse_u64(&bytes[..eq])?;
    let tag = u32::try_from(tag).ok()?;
    let after = &bytes[eq + 1..];
    let end = after.iter().position(|&b| b == SOH)?;
    Some((Field { tag, value: &after[..end] }, eq + 1 + end + 1))
}

/// Parses ASCII digits into `u64` (no sign, no whitespace, ≤ 19 digits).
#[inline]
pub fn parse_u64(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() || bytes.len() > 19 {
        return None;
    }
    let mut v: u64 = 0;
    for &b in bytes {
        let d = b.wrapping_sub(b'0');
        if d > 9 {
            return None;
        }
        v = v * 10 + d as u64;
    }
    Some(v)
}

/// Why a decimal string could not be scaled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecimalError {
    /// Not `[-]digits[.digits]`, or overflow.
    Malformed,

    /// More non-zero fractional digits than the scale keeps.
    Precision,
}

/// Parses `[-]digits[.digits]` into an integer scaled by `10^scale`.
///
/// Exact: `1450.10` on `S2` is `145010`; `1450.105` on `S2` is
/// [`DecimalError::Precision`]. Trailing fractional zeros beyond the scale
/// are accepted. A leading `+` is not FIX and is rejected.
#[inline]
pub fn parse_scaled_decimal(bytes: &[u8], scale: Scale) -> Result<i64, DecimalError> {
    let (negative, digits) = match bytes {
        [b'-', rest @ ..] => (true, rest),
        _ => (false, bytes),
    };
    if digits.is_empty() {
        return Err(DecimalError::Malformed);
    }
    let (int_part, frac_part) = match digits.iter().position(|&b| b == b'.') {
        Some(i) => (&digits[..i], &digits[i + 1..]),
        None => (digits, &[][..]),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return Err(DecimalError::Malformed);
    }
    let decimals = scale.decimals() as usize;
    let mut v: i64 = 0;
    for &b in int_part {
        let d = b.wrapping_sub(b'0');
        if d > 9 {
            return Err(DecimalError::Malformed);
        }
        v = v
            .checked_mul(10)
            .and_then(|x| x.checked_add(d as i64))
            .ok_or(DecimalError::Malformed)?;
    }
    let mut used = 0usize;
    for &b in frac_part {
        let d = b.wrapping_sub(b'0');
        if d > 9 {
            return Err(DecimalError::Malformed);
        }
        if used < decimals {
            v = v
                .checked_mul(10)
                .and_then(|x| x.checked_add(d as i64))
                .ok_or(DecimalError::Malformed)?;
            used += 1;
        } else if d != 0 {
            return Err(DecimalError::Precision);
        }
    }
    for _ in used..decimals {
        v = v.checked_mul(10).ok_or(DecimalError::Malformed)?;
    }
    Ok(if negative { -v } else { v })
}

/// Nanoseconds per second.
const NS_PER_SEC: u64 = 1_000_000_000;

/// Parses `YYYYMMDD-HH:MM:SS[.f{1,9}]` into UTC unix nanoseconds.
#[inline]
pub fn parse_utc_timestamp(bytes: &[u8]) -> Option<UnixNano> {
    if bytes.len() < 17 || bytes[8] != b'-' {
        return None;
    }
    let date = parse_utc_date(&bytes[..8])?;
    let time = parse_utc_time(&bytes[9..])?;
    Some(date + time)
}

/// Parses `YYYYMMDD` into UTC unix nanoseconds at midnight.
#[inline]
pub fn parse_utc_date(bytes: &[u8]) -> Option<UnixNano> {
    if bytes.len() != 8 {
        return None;
    }
    let y = parse_u64(&bytes[..4])? as i64;
    let m = parse_u64(&bytes[4..6])? as u32;
    let d = parse_u64(&bytes[6..8])? as u32;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let days = days_from_civil(y, m, d);
    let secs = u64::try_from(days).ok()?.checked_mul(86_400)?;
    secs.checked_mul(NS_PER_SEC)
}

/// Parses `HH:MM:SS[.f{1,9}]` into nanoseconds since midnight.
#[inline]
pub fn parse_utc_time(bytes: &[u8]) -> Option<UnixNano> {
    if bytes.len() < 8 || bytes[2] != b':' || bytes[5] != b':' {
        return None;
    }
    let h = parse_u64(&bytes[..2])?;
    let m = parse_u64(&bytes[3..5])?;
    let s = parse_u64(&bytes[6..8])?;
    if h > 23 || m > 59 || s > 60 {
        return None;
    }
    let frac_ns = match &bytes[8..] {
        [] => 0,
        [b'.', frac @ ..] if !frac.is_empty() && frac.len() <= 9 => {
            let v = parse_u64(frac)?;
            v * 10u64.pow(9 - frac.len() as u32)
        }
        _ => return None,
    };
    Some((h * 3_600 + m * 60 + s) * NS_PER_SEC + frac_ns)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
#[inline]
pub const fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `MsgType` (tag 35) values this parser distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MsgType {
    /// `0` — session liveness.
    Heartbeat,

    /// `1` — asks the counterparty for a heartbeat.
    TestRequest,

    /// `2` — asks for retransmission of a sequence range.
    ResendRequest,

    /// `3` — session-level reject.
    Reject,

    /// `4` — sequence reset / gap fill.
    SequenceReset,

    /// `5` — logout.
    Logout,

    /// `A` — logon.
    Logon,

    /// `V` — market data request (outbound).
    MarketDataRequest,

    /// `W` — full book snapshot.
    MarketDataSnapshot,

    /// `X` — incremental refresh.
    MarketDataIncremental,

    /// `Y` — market data request reject.
    MarketDataRequestReject,

    /// Anything else, identified by its first byte.
    Other {
        /// First byte of the raw value.
        first: u8,
    },
}

impl MsgType {
    /// Classifies a raw `35=` value.
    #[inline]
    pub const fn from_value(value: &[u8]) -> Self {
        match value {
            b"0" => Self::Heartbeat,
            b"1" => Self::TestRequest,
            b"2" => Self::ResendRequest,
            b"3" => Self::Reject,
            b"4" => Self::SequenceReset,
            b"5" => Self::Logout,
            b"A" => Self::Logon,
            b"V" => Self::MarketDataRequest,
            b"W" => Self::MarketDataSnapshot,
            b"X" => Self::MarketDataIncremental,
            b"Y" => Self::MarketDataRequestReject,
            [first, ..] => Self::Other { first: *first },
            [] => Self::Other { first: 0 },
        }
    }

    /// `true` for `W` and `X`.
    #[inline]
    pub const fn is_market_data(self) -> bool {
        matches!(self, Self::MarketDataSnapshot | Self::MarketDataIncremental)
    }

    /// `true` for the session-administration types (`0 1 2 3 4 5 A`).
    #[inline]
    pub const fn is_admin(self) -> bool {
        matches!(
            self,
            Self::Heartbeat
                | Self::TestRequest
                | Self::ResendRequest
                | Self::Reject
                | Self::SequenceReset
                | Self::Logout
                | Self::Logon
        )
    }
}
