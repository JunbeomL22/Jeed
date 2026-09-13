//! Value types that appear **on the wire**.
//!
//! These live here rather than in a separate crate because they are header
//! and payload fields, not general-purpose domain types: `venue`, `isin`,
//! `price_scale` and `qty_scale` are bytes in [`RecordHeader`], and
//! [`BookPrice`] / [`BookQuantity`] are the integer widths of every price and
//! size the ABI carries.
//!
//! Deliberately absent: process-local interned identifiers. An interning
//! counter is per-process, so the same ISIN gets a different id in the feed
//! handler and in the consumer — identity crosses the boundary as raw
//! `(venue, isin)` and the consumer resolves it (`documents/feed_handler.md` §6).
//!
//! [`RecordHeader`]: crate::RecordHeader

/// Length of an ISIN code in bytes.
pub const ISIN_LEN: usize = 12;

/// Raw ISIN bytes, exactly as the venue sent them.
///
/// For venues that quote something other than a listed security (an FX pair,
/// say) this carries the venue's own symbol, left-aligned and space-padded.
pub type Isin = [u8; ISIN_LEN];

/// Price in `price_scale` units. Signed — yields and spreads go negative.
pub type BookPrice = i64;

/// Quantity in `qty_scale` units.
pub type BookQuantity = u64;

/// Bond yield in basis-point-ish fixed point; carried in a level's `ext` word.
pub type BookYield = i32;

/// Count of resting orders at a level.
pub type OrderCount = u32;

/// Nanoseconds since the Unix epoch.
///
/// **Not monotonic**, and unsigned: `a - b` wraps. Every difference must go
/// through `saturating_sub` (`documents/feed_handler.md` §6).
pub type UnixNano = u64;

/// Trading venue. Field-less so it is one byte on the wire.
///
/// The discriminants are part of the ABI: changing one is a wire format
/// change, not a rename.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum Venue {
    /// Korea Exchange.
    #[default]
    Krx = 0,

    /// Nextrade (대체거래소, NXT).
    Nxt = 1,

    /// 서울외국환중개 (Seoul Money Brokerage Services) — USDKRW spot.
    Smbs = 2,
}

impl Venue {
    /// Short ASCII tag.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Krx => "KRX",
            Self::Nxt => "NXT",
            Self::Smbs => "SMBS",
        }
    }

    /// Wire byte.
    #[inline]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Inverse of [`as_u8`](Self::as_u8).
    #[inline]
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Krx),
            1 => Some(Self::Nxt),
            2 => Some(Self::Smbs),
            _ => None,
        }
    }

    /// Parses a venue from its ASCII tag, case-sensitive.
    #[inline]
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "KRX" => Some(Self::Krx),
            "NXT" => Some(Self::Nxt),
            "SMBS" => Some(Self::Smbs),
            _ => None,
        }
    }
}

impl core::fmt::Display for Venue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Decimal scale of an integer price or quantity: the value is
/// `raw / 10^decimals`.
///
/// The discriminant *is* the decimal count, so the wire byte and
/// [`decimals`](Self::decimals) are the same number.
///
/// Scales belong on the wire because the integer conversion happens at the
/// parse site: the feed handler turns venue text into integers once, and the
/// rounding point never moves afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum Scale {
    /// Integer (10^0).
    #[default]
    S0 = 0,
    /// 1 decimal place.
    S1 = 1,
    /// 2 decimal places.
    S2 = 2,
    /// 3 decimal places.
    S3 = 3,
    /// 4 decimal places.
    S4 = 4,
    /// 5 decimal places.
    S5 = 5,
    /// 6 decimal places.
    S6 = 6,
    /// 7 decimal places.
    S7 = 7,
    /// 8 decimal places.
    S8 = 8,
}

/// `10^n` for `n` in `0..=8`.
const DIVISORS: [u64; 9] = [1, 10, 100, 1_000, 10_000, 100_000, 1_000_000, 10_000_000, 100_000_000];

/// `10^-n` for `n` in `0..=8`.
const MULTIPLIERS: [f64; 9] = [1.0, 1e-1, 1e-2, 1e-3, 1e-4, 1e-5, 1e-6, 1e-7, 1e-8];

impl Scale {
    /// Scale for a decimal count. `None` past 8 places.
    #[inline]
    pub const fn from_decimals(decimals: usize) -> Option<Self> {
        match decimals {
            0 => Some(Self::S0),
            1 => Some(Self::S1),
            2 => Some(Self::S2),
            3 => Some(Self::S3),
            4 => Some(Self::S4),
            5 => Some(Self::S5),
            6 => Some(Self::S6),
            7 => Some(Self::S7),
            8 => Some(Self::S8),
            _ => None,
        }
    }

    /// Number of decimal places — also the wire byte.
    #[inline]
    pub const fn decimals(self) -> u8 {
        self as u8
    }

    /// `10^decimals`.
    #[inline]
    pub const fn divisor(self) -> u64 {
        DIVISORS[self as usize]
    }

    /// `10^-decimals`, for display only.
    ///
    /// Nothing on the hot path should convert to `f64`; the integer and its
    /// scale are the value.
    #[inline]
    pub const fn multiplier(self) -> f64 {
        MULTIPLIERS[self as usize]
    }
}

impl core::fmt::Display for Scale {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "S{}", self.decimals())
    }
}
