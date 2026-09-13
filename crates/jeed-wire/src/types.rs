//! Value types that appear **on the wire**.
//!
//! These live here rather than in a separate crate because they are header
//! and payload fields, not general-purpose domain types: `venue`, `isin`,
//! `price_scale` and `qty_scale` are bytes in [`RecordHeader`], and
//! [`BookPrice`] / [`BookQuantity`] are the integer widths of every price and
//! size the ABI carries.
//!
//! Deliberately absent: process-local interned identifiers. An interning
//! counter is per-process, so the same instrument gets a different id in the
//! feed handler and in the consumer — identity crosses the boundary as raw
//! `(venue, symbol)` and the consumer resolves it
//! (`documents/feed_handler.md` §6).
//!
//! [`RecordHeader`]: crate::RecordHeader

/// Width of the header's symbol field in bytes.
///
/// Twenty-four rather than an ISIN's twelve because the venue's own name for
/// the instrument is what crosses the boundary, and not every venue names
/// instruments in twelve bytes: Binance USD-M lists `1000000MOGUSDT` (14) and
/// an OKX option is `BTC-USD-240329-70000-C` (22). A KRX ISIN uses the first
/// twelve and leaves the rest `NUL`.
pub const SYMBOL_LEN: usize = 24;

/// The venue's own name for the instrument, exactly as it sent it:
/// left-aligned and `NUL`-padded.
///
/// For KRX this is the twelve-byte ISIN. For a crypto venue it is the venue
/// symbol in the venue's own casing (`BTCUSDT`, `KRW-BTC`) — **not** a
/// normalised pair name, because normalising is a mapping and a mapping is
/// the consumer's (`documents/feed_handler.md` §6).
///
/// `NUL` rather than space padding so that an all-zero slot stays all-zero and
/// the trailing pad is unambiguous: no venue puts a `NUL` in a symbol.
pub type Symbol = [u8; SYMBOL_LEN];

/// Builds a [`Symbol`] from a venue symbol, `NUL`-padding the tail.
///
/// `None` if the input is empty, longer than [`SYMBOL_LEN`], or contains a
/// `NUL` (which would make the padding ambiguous).
#[inline]
pub const fn symbol_from_bytes(bytes: &[u8]) -> Option<Symbol> {
    if bytes.is_empty() || bytes.len() > SYMBOL_LEN {
        return None;
    }
    let mut out = [0u8; SYMBOL_LEN];
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0 {
            return None;
        }
        out[i] = bytes[i];
        i += 1;
    }
    Some(out)
}

/// The symbol without its `NUL` padding.
#[inline]
pub fn symbol_bytes(symbol: &Symbol) -> &[u8] {
    let len = symbol.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    &symbol[..len]
}

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

    /// Binance spot.
    BinanceSpot = 3,

    /// Binance USD-M futures.
    ///
    /// A separate venue from [`BinanceSpot`](Self::BinanceSpot) rather than a
    /// market flag beside it, because `BTCUSDT` names a different instrument
    /// on each and `(venue, symbol)` is the identity. COIN-M, when it arrives,
    /// takes a byte of its own for the same reason.
    BinanceFutures = 4,

    /// Upbit (spot only — Korean law does not let a domestic exchange list
    /// crypto derivatives).
    Upbit = 5,

    /// Bithumb (spot only, same reason).
    ///
    /// Its own byte even though the message format is Upbit's: the two are
    /// different order books with different prices, and `KRW-BTC` is the
    /// symbol on both. Sharing a byte would merge them.
    Bithumb = 6,

    /// OKX — every market, on one byte.
    ///
    /// The exception to the rule the Binance and Bybit entries follow, because
    /// OKX's `instId` already separates the markets: `BTC-USDT` is spot,
    /// `BTC-USDT-SWAP` the perpetual, `BTC-USDT-240329` the future and
    /// `BTC-USD-240329-70000-C` an option. There is no collision to break.
    Okx = 7,

    /// Bybit spot.
    BybitSpot = 8,

    /// Bybit USDT/USDC linear perpetuals and futures.
    ///
    /// Split from [`BybitSpot`](Self::BybitSpot) on the Binance grounds and
    /// not the OKX ones: Bybit spells the linear perpetual `BTCUSDT`, exactly
    /// as it spells the spot pair, and only the endpoint tells them apart.
    /// Inverse contracts, when they arrive, take a third byte.
    BybitLinear = 9,

    /// Bitget spot.
    BitgetSpot = 10,

    /// Bitget USDT/USDC-margined futures (`USDT-FUTURES`, `USDC-FUTURES`).
    ///
    /// The Bybit case again: `BTCUSDT` is the spot pair and the perpetual, and
    /// only the subscription's `instType` says which. Coin-margined
    /// (`COIN-FUTURES`) takes a third byte when it arrives.
    BitgetLinear = 11,

    /// Gate.io spot.
    ///
    /// Named for the market rather than the venue because Gate's perpetuals
    /// spell the contract exactly as spot spells the pair (`BTC_USDT`), so the
    /// second market cannot share this byte when it is ported.
    GateSpot = 12,

    /// KuCoin spot.
    KucoinSpot = 13,

    /// KuCoin futures.
    ///
    /// Split from [`KucoinSpot`](Self::KucoinSpot) even though today's symbols
    /// cannot collide — futures renames the asset and suffixes the contract
    /// (`XBTUSDTM` against spot's `BTC-USDT`). That is a naming habit, not a
    /// rule KuCoin documents, and the cost of being wrong about it is two
    /// order books merged into one.
    KucoinFutures = 14,
}

impl Venue {
    /// Short ASCII tag.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Krx => "KRX",
            Self::Nxt => "NXT",
            Self::Smbs => "SMBS",
            Self::BinanceSpot => "BINANCE_SPOT",
            Self::BinanceFutures => "BINANCE_FUTURES",
            Self::Upbit => "UPBIT",
            Self::Bithumb => "BITHUMB",
            Self::Okx => "OKX",
            Self::BybitSpot => "BYBIT_SPOT",
            Self::BybitLinear => "BYBIT_LINEAR",
            Self::BitgetSpot => "BITGET_SPOT",
            Self::BitgetLinear => "BITGET_LINEAR",
            Self::GateSpot => "GATE_SPOT",
            Self::KucoinSpot => "KUCOIN_SPOT",
            Self::KucoinFutures => "KUCOIN_FUTURES",
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
            3 => Some(Self::BinanceSpot),
            4 => Some(Self::BinanceFutures),
            5 => Some(Self::Upbit),
            6 => Some(Self::Bithumb),
            7 => Some(Self::Okx),
            8 => Some(Self::BybitSpot),
            9 => Some(Self::BybitLinear),
            10 => Some(Self::BitgetSpot),
            11 => Some(Self::BitgetLinear),
            12 => Some(Self::GateSpot),
            13 => Some(Self::KucoinSpot),
            14 => Some(Self::KucoinFutures),
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
            "BINANCE_SPOT" => Some(Self::BinanceSpot),
            "BINANCE_FUTURES" => Some(Self::BinanceFutures),
            "UPBIT" => Some(Self::Upbit),
            "BITHUMB" => Some(Self::Bithumb),
            "OKX" => Some(Self::Okx),
            "BYBIT_SPOT" => Some(Self::BybitSpot),
            "BYBIT_LINEAR" => Some(Self::BybitLinear),
            "BITGET_SPOT" => Some(Self::BitgetSpot),
            "BITGET_LINEAR" => Some(Self::BitgetLinear),
            "GATE_SPOT" => Some(Self::GateSpot),
            "KUCOIN_SPOT" => Some(Self::KucoinSpot),
            "KUCOIN_FUTURES" => Some(Self::KucoinFutures),
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
