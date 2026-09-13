//! Record kinds and flag bit definitions.
//!
//! Flags carry **conclusions, not evidence** (`documents/feed_handler.md` §8):
//! a bit the consumer could only interpret by branching on the venue does not
//! belong here. Handler-internal facts (exchange sequence gaps, resend state,
//! heartbeat cadence, A/B dedup) never reach the wire.

use crate::error::WireError;

/// Record kind. `0` is deliberately unused so an all-zero slot is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum WireKind {
    /// Full book snapshot ([`QuotePayload`](crate::QuotePayload)).
    Quote = 1,

    /// Single trade print ([`TradePayload`](crate::TradePayload)).
    Trade = 2,

    /// Trade print plus the post-trade book
    /// ([`TradeQuotePayload`](crate::TradeQuotePayload)).
    TradeQuote = 3,

    /// Open interest ([`OpenInterestPayload`](crate::OpenInterestPayload)).
    OpenInterest = 4,

    /// Investor statistics ([`InvestorStatsPayload`](crate::InvestorStatsPayload)).
    InvestorStats = 5,

    /// Incremental book delta. **Reserved** — the payload is defined when a
    /// delta feed is attached.
    SnapshotDelta = 6,

    /// Applied price-limit expansion ([`PriceLimitPayload`](crate::PriceLimitPayload)).
    PriceLimit = 7,

    /// Market operation notice ([`MarketSchedulePayload`](crate::MarketSchedulePayload)).
    MarketSchedule = 8,

    /// Producer liveness ([`HeartbeatPayload`](crate::HeartbeatPayload)).
    /// Emitted from the receive loop itself, never from a side thread (§9).
    Heartbeat = 9,

    /// Intraday dynamic price band applied or released
    /// ([`DynamicPriceLimitPayload`](crate::DynamicPriceLimitPayload)).
    ///
    /// Separate from [`PriceLimit`](Self::PriceLimit) because the two are
    /// different fences: that one is the daily band that widens in stages,
    /// this one moves with every print. An order must clear both.
    ///
    /// It is also the only way to learn that the band was **released**. A
    /// release comes with no trade, so a consumer watching only the band
    /// carried on `Trade`/`TradeQuote` records would keep believing the last
    /// one it saw.
    DynamicPriceLimit = 10,
}

impl WireKind {
    /// Decodes a raw kind byte.
    #[inline]
    pub const fn from_u8(v: u8) -> Result<Self, WireError> {
        Ok(match v {
            1 => Self::Quote,
            2 => Self::Trade,
            3 => Self::TradeQuote,
            4 => Self::OpenInterest,
            5 => Self::InvestorStats,
            6 => Self::SnapshotDelta,
            7 => Self::PriceLimit,
            8 => Self::MarketSchedule,
            9 => Self::Heartbeat,
            10 => Self::DynamicPriceLimit,
            found => return Err(WireError::Kind { found }),
        })
    }

    /// Raw kind byte.
    #[inline]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// `true` for kinds whose next message fully replaces the previous one.
    ///
    /// A ring drop of a self-healing kind only needs a counter; a drop of a
    /// non-self-healing kind ([`SnapshotDelta`](Self::SnapshotDelta)) must
    /// trigger recovery (§7).
    #[inline]
    pub const fn is_self_healing(self) -> bool {
        !matches!(self, Self::SnapshotDelta)
    }
}

/// Header flag bits ([`RecordHeader::flags`](crate::RecordHeader::flags)).
pub mod header_flags {
    /// `venue_ns` holds a real exchange timestamp.
    pub const VENUE_TIME_VALID: u8 = 1 << 0;

    /// Producer judged this data stale; the consumer must not act on it.
    /// How staleness was judged is handler-internal (§8).
    pub const STALE: u8 = 1 << 1;

    /// Bid side carries no levels.
    pub const BID_EMPTY: u8 = 1 << 2;

    /// Ask side carries no levels.
    pub const ASK_EMPTY: u8 = 1 << 3;
}

/// Quote payload level-flag bits ([`QuotePayload::level_flags`](crate::QuotePayload::level_flags)).
pub mod level_flags {
    /// Every level's `order_count` is meaningful (derivative channels).
    pub const ORDER_COUNT_VALID: u8 = 1 << 0;
}

/// Encoding of the per-level `ext` word
/// ([`QuotePayload::level_ext_kind`](crate::QuotePayload::level_ext_kind)).
pub mod level_ext {
    /// `ext` is unused.
    pub const NONE: u8 = 0;

    /// `ext` is a bond yield (`i32` bit pattern).
    pub const BOND_YIELD: u8 = 1;

    /// `ext` is the liquidity-provider quantity at that level (`u32`).
    pub const LP_QUANTITY: u8 = 2;
}

/// Encoding of the quote-level `quote_ext` word
/// ([`QuotePayload::quote_ext_kind`](crate::QuotePayload::quote_ext_kind)).
pub mod quote_ext {
    /// `quote_ext` is unused.
    pub const NONE: u8 = 0;

    /// `quote_ext` is the total LP holdings (KRX equity).
    pub const LP_HOLDINGS: u8 = 1;

    /// `quote_ext` is the exchange sequence boundary (crypto snapshots).
    pub const SEQUENCE: u8 = 2;

    /// `quote_ext` is the indicative price of a call auction before the cross
    /// (KRX 예상체결가), in the record's `price_scale`.
    ///
    /// Only meaningful while an auction is running; the field is zero
    /// otherwise, and a zero is not published.
    pub const EXPECTED_PRICE: u8 = 3;
}

/// Trade payload flag bits ([`TradePayload::trade_flags`](crate::TradePayload::trade_flags)).
pub mod trade_flags {
    /// `cumulative_qty` is meaningful.
    pub const CUMULATIVE_QTY_VALID: u8 = 1 << 0;

    /// `trade_yield` is meaningful (bond channels).
    pub const YIELD_VALID: u8 = 1 << 1;

    /// `dyn_upper` / `dyn_lower` are meaningful.
    ///
    /// Separate from the values because KRX cannot express "not applicable":
    /// instruments outside the dynamic-limit regime carry `000000.00`, not
    /// blanks, so a zero band is indistinguishable from an absent one in the
    /// raw message. The handler decides; the wire carries the conclusion (§8).
    pub const DYN_LIMIT_VALID: u8 = 1 << 2;
}

/// Trade kind encoding ([`TradePayload::trade_kind`](crate::TradePayload::trade_kind)).
///
/// [`trade_kind::NONE`] exists so that "this channel carries no aggressor flag" is a
/// distinct value from "the flag was present but said nothing".
pub mod trade_kind {
    /// Channel carries no aggressor flag.
    pub const NONE: u8 = 0;

    /// Aggressive seller.
    pub const SELL: u8 = 1;

    /// Aggressive buyer.
    pub const BUY: u8 = 2;

    /// Flag present but unclassified.
    pub const UNKNOWN: u8 = 3;
}

/// 동적가격제한설정코드 encoding
/// ([`DynamicPriceLimitPayload::action`](crate::DynamicPriceLimitPayload::action)).
///
/// The raw KRX byte is not carried: whether a band is in force is a conclusion,
/// and the consumer should not have to know one venue's codebook to read it
/// (§8).
pub mod dyn_limit_action {
    /// The message said something this build does not recognise. The band in
    /// the payload must not be acted on.
    pub const UNKNOWN: u8 = 0;

    /// A band is in force; `upper_price` / `lower_price` are it.
    pub const APPLIED: u8 = 1;

    /// The band is lifted. Prices in the payload are not a band.
    pub const RELEASED: u8 = 2;
}

/// Market schedule flag bits
/// ([`MarketSchedulePayload::schedule_flags`](crate::MarketSchedulePayload::schedule_flags)).
pub mod schedule_flags {
    /// The header ISIN scopes this notice to one instrument
    /// (clear ⇒ product/group-level notice, ISIN is twelve spaces).
    pub const INSTRUMENT_SCOPED: u8 = 1 << 0;

    /// `expected_time_of_day` is meaningful.
    pub const EXPECTED_TIME_VALID: u8 = 1 << 1;
}

/// Expansion direction encoding
/// ([`MarketSchedulePayload::expansion_direction`](crate::MarketSchedulePayload::expansion_direction)).
pub mod expansion_direction {
    /// Notice does not announce an expansion condition.
    pub const NOT_APPLICABLE: u8 = 0;

    /// Upward expansion condition occurred.
    pub const UP: u8 = 1;

    /// Downward expansion condition occurred.
    pub const DOWN: u8 = 2;
}
