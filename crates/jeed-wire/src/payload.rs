//! Per-kind payloads and the payload union.
//!
//! Every payload is plain-old-data: integers and byte arrays only, explicit
//! padding, `#[repr(C)]`. That is what makes reading any member of
//! [`WirePayload`] sound regardless of which member was last written — every
//! bit pattern is a valid value of every member.

use crate::kind::{level_flags, trade_flags, trade_kind};
use crate::types::{BookPrice, BookQuantity, BookYield, OrderCount};
use crate::{WIRE_MAX_DEPTH, WIRE_PAYLOAD_LEN};
use core::fmt;
use core::mem::size_of;

// ============================================================================
// Book level
// ============================================================================

/// One book level (24 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct WireLevel {
    /// Price in `price_scale` units.
    pub price: BookPrice,

    /// Quantity in `qty_scale` units.
    pub qty: BookQuantity,

    /// Resting-order count; meaningful only with
    /// [`level_flags::ORDER_COUNT_VALID`].
    pub order_count: OrderCount,

    /// Extension word; meaning given by [`QuotePayload::level_ext_kind`].
    pub ext: u32,
}

const _: () = assert!(size_of::<WireLevel>() == 24);
const _: () = assert!(
    size_of::<BookPrice>() + size_of::<BookQuantity>() + size_of::<OrderCount>() + size_of::<u32>()
        == size_of::<WireLevel>()
);

impl WireLevel {
    /// Level with price and quantity only.
    #[inline]
    pub const fn new(price: BookPrice, qty: BookQuantity) -> Self {
        Self { price, qty, order_count: 0, ext: 0 }
    }

    /// Level with a resting-order count.
    #[inline]
    pub const fn with_count(price: BookPrice, qty: BookQuantity, order_count: OrderCount) -> Self {
        Self { price, qty, order_count, ext: 0 }
    }

    /// Bond yield view of `ext` ([`level_ext::BOND_YIELD`](crate::level_ext::BOND_YIELD)).
    #[inline]
    pub const fn bond_yield(&self) -> BookYield {
        self.ext as i32
    }
}

// ============================================================================
// Quote
// ============================================================================

/// Full book snapshot, both sides, level 0 innermost (496 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct QuotePayload {
    /// Ask levels, ascending price. Entries past the header `depth` are zero.
    pub ask: [WireLevel; WIRE_MAX_DEPTH],

    /// Bid levels, descending price. Entries past the header `depth` are zero.
    pub bid: [WireLevel; WIRE_MAX_DEPTH],

    /// Quote-level extension word; meaning given by `quote_ext_kind`.
    pub quote_ext: u64,

    /// [`level_ext`](crate::level_ext) encoding of every level's `ext` word.
    pub level_ext_kind: u8,

    /// [`quote_ext`](crate::quote_ext) encoding of `quote_ext`.
    pub quote_ext_kind: u8,

    /// [`level_flags`] bits.
    pub level_flags: u8,

    /// Explicit padding — always zero.
    pub _pad: [u8; 5],
}

const _: () = assert!(size_of::<QuotePayload>() == 496);
const _: () = assert!(
    size_of::<WireLevel>() * WIRE_MAX_DEPTH * 2 + size_of::<u64>() + 3 * size_of::<u8>() + 5
        == size_of::<QuotePayload>()
);

impl QuotePayload {
    /// Sets one ask level.
    #[inline]
    pub const fn set_ask(&mut self, i: usize, level: WireLevel) -> &mut Self {
        self.ask[i] = level;
        self
    }

    /// Sets one bid level.
    #[inline]
    pub const fn set_bid(&mut self, i: usize, level: WireLevel) -> &mut Self {
        self.bid[i] = level;
        self
    }

    /// Marks every level's `order_count` as meaningful.
    #[inline]
    pub const fn with_order_counts(&mut self) -> &mut Self {
        self.level_flags |= level_flags::ORDER_COUNT_VALID;
        self
    }

    /// `true` when `order_count` is meaningful.
    #[inline]
    pub const fn has_order_counts(&self) -> bool {
        self.level_flags & level_flags::ORDER_COUNT_VALID != 0
    }

    /// Declares the per-level `ext` word encoding.
    #[inline]
    pub const fn with_level_ext(&mut self, kind: u8) -> &mut Self {
        self.level_ext_kind = kind;
        self
    }

    /// Sets the quote-level extension.
    #[inline]
    pub const fn with_quote_ext(&mut self, kind: u8, value: u64) -> &mut Self {
        self.quote_ext_kind = kind;
        self.quote_ext = value;
        self
    }

    /// Ask levels up to `depth`.
    #[inline]
    pub fn asks(&self, depth: u8) -> &[WireLevel] {
        &self.ask[..(depth as usize).min(WIRE_MAX_DEPTH)]
    }

    /// Bid levels up to `depth`.
    #[inline]
    pub fn bids(&self, depth: u8) -> &[WireLevel] {
        &self.bid[..(depth as usize).min(WIRE_MAX_DEPTH)]
    }
}

// ============================================================================
// Trade
// ============================================================================

/// One trade print (32 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct TradePayload {
    /// Trade price in `price_scale` units.
    pub price: BookPrice,

    /// Trade quantity in `qty_scale` units.
    pub qty: BookQuantity,

    /// Session-cumulative quantity; meaningful with
    /// [`trade_flags::CUMULATIVE_QTY_VALID`].
    pub cumulative_qty: BookQuantity,

    /// Dynamic upper price limit in force after this print, in `price_scale`
    /// units; meaningful with [`trade_flags::DYN_LIMIT_VALID`].
    pub dyn_upper: BookPrice,

    /// Dynamic lower price limit in force after this print; meaningful with
    /// [`trade_flags::DYN_LIMIT_VALID`].
    pub dyn_lower: BookPrice,

    /// Trade yield; meaningful with [`trade_flags::YIELD_VALID`].
    pub trade_yield: BookYield,

    /// [`trade_kind`] encoding.
    pub trade_kind: u8,

    /// [`trade_flags`] bits.
    pub trade_flags: u8,

    /// Explicit padding — always zero.
    pub _pad: u16,
}

const _: () = assert!(size_of::<TradePayload>() == 48);
const _: () = assert!(
    3 * size_of::<BookPrice>()
        + 2 * size_of::<BookQuantity>()
        + size_of::<BookYield>()
        + 2 * size_of::<u8>()
        + size_of::<u16>()
        == size_of::<TradePayload>()
);

impl TradePayload {
    /// Trade with price and quantity only ([`trade_kind::NONE`]).
    #[inline]
    pub const fn new(price: BookPrice, qty: BookQuantity) -> Self {
        Self {
            price,
            qty,
            cumulative_qty: 0,
            dyn_upper: 0,
            dyn_lower: 0,
            trade_yield: 0,
            trade_kind: trade_kind::NONE,
            trade_flags: 0,
            _pad: 0,
        }
    }

    /// Sets the aggressor encoding.
    #[inline]
    pub const fn with_kind(&mut self, kind: u8) -> &mut Self {
        self.trade_kind = kind;
        self
    }

    /// Sets the cumulative quantity and marks it valid.
    #[inline]
    pub const fn with_cumulative_qty(&mut self, cumulative: BookQuantity) -> &mut Self {
        self.cumulative_qty = cumulative;
        self.trade_flags |= trade_flags::CUMULATIVE_QTY_VALID;
        self
    }

    /// Sets the dynamic price limits in force and marks them valid.
    ///
    /// KRX sends `000000.00` rather than blanks for instruments the dynamic
    /// limit regime does not cover (far-month futures, spreads). Zero is
    /// therefore not distinguishable from "not applicable" in the raw message,
    /// which is why the conclusion rides in a flag instead of the value
    /// (`CLAUDE.md`).
    #[inline]
    pub const fn with_dyn_limits(&mut self, upper: BookPrice, lower: BookPrice) -> &mut Self {
        self.dyn_upper = upper;
        self.dyn_lower = lower;
        self.trade_flags |= trade_flags::DYN_LIMIT_VALID;
        self
    }

    /// Dynamic `(upper, lower)` price limits, if the producer marked them
    /// applicable.
    ///
    /// These are the **inner** fence of the order-eligible range; the outer one
    /// is the static price limit carried by `V1`
    /// ([`PriceLimitPayload`]). An order must clear both
    /// (`documents/krx/실시간가격제한.md`).
    #[inline]
    pub const fn dyn_limits(&self) -> Option<(BookPrice, BookPrice)> {
        if self.trade_flags & trade_flags::DYN_LIMIT_VALID != 0 {
            Some((self.dyn_upper, self.dyn_lower))
        } else {
            None
        }
    }

    /// Sets the yield and marks it valid.
    #[inline]
    pub const fn with_yield(&mut self, trade_yield: BookYield) -> &mut Self {
        self.trade_yield = trade_yield;
        self.trade_flags |= trade_flags::YIELD_VALID;
        self
    }

    /// Cumulative quantity, if valid.
    #[inline]
    pub const fn cumulative_qty(&self) -> Option<BookQuantity> {
        if self.trade_flags & trade_flags::CUMULATIVE_QTY_VALID != 0 {
            Some(self.cumulative_qty)
        } else {
            None
        }
    }

    /// Yield, if valid.
    #[inline]
    pub const fn trade_yield(&self) -> Option<BookYield> {
        if self.trade_flags & trade_flags::YIELD_VALID != 0 {
            Some(self.trade_yield)
        } else {
            None
        }
    }
}

// ============================================================================
// Trade + quote
// ============================================================================

/// Trade print followed by the post-trade book (544 bytes).
///
/// This is the largest payload, so it sets [`WIRE_PAYLOAD_LEN`]. KRX
/// `IFMSRPD0037` (`G7`) is the channel that produces it: the print and the
/// book it left behind arrive in one message, so they cannot be split by
/// reordering or loss the way a separate trade and quote can.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct TradeQuotePayload {
    /// The print.
    pub trade: TradePayload,

    /// The book after the print.
    pub quote: QuotePayload,
}

const _: () = assert!(size_of::<TradeQuotePayload>() == 544);
const _: () =
    assert!(size_of::<TradePayload>() + size_of::<QuotePayload>() == size_of::<TradeQuotePayload>());

// ============================================================================
// Small payloads
// ============================================================================

/// Open interest (8 bytes). Quantity scale is the header `qty_scale`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct OpenInterestPayload {
    /// Open interest in `qty_scale` units.
    pub open_interest: BookQuantity,
}

const _: () = assert!(size_of::<OpenInterestPayload>() == 8);

/// Investor statistics (48 bytes). Header `qty_scale` scales the quantities,
/// header `price_scale` scales the values. Product / investor kinds are
/// carried as the raw KRX code bytes; the consumer maps them to its own ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct InvestorStatsPayload {
    /// Buy quantity.
    pub buy_qty: BookQuantity,

    /// Sell quantity.
    pub sell_qty: BookQuantity,

    /// Buy value.
    pub buy_value: BookQuantity,

    /// Sell value.
    pub sell_value: BookQuantity,

    /// Raw KRX product-kind code (11 bytes).
    pub product_kind: [u8; 11],

    /// Raw KRX derivative-kind byte.
    pub derivative_kind: u8,

    /// Raw KRX investor-kind code (4 bytes).
    pub investor_kind: [u8; 4],
}

const _: () = assert!(size_of::<InvestorStatsPayload>() == 48);
const _: () = assert!(4 * size_of::<BookQuantity>() + 11 + 1 + 4 == size_of::<InvestorStatsPayload>());

/// Applied price-limit expansion (40 bytes). Price scale is the header
/// `price_scale`; the instrument is the header ISIN.
///
/// This is the *daily* limit band (KRX `V1` / `IFMSRPD0043`), which widens in
/// discrete stages — hence `upper_stage` / `lower_stage`, which move
/// independently. It is a different axis from the intraday dynamic band; see
/// `documents/krx/실시간가격제한.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct PriceLimitPayload {
    /// Exchange application time, ns since KST midnight.
    pub applied_time_of_day: u64,

    /// Applied upper price.
    pub upper_price: BookPrice,

    /// Applied lower price.
    pub lower_price: BookPrice,

    /// Exchange distribution sequence. **May be zero because the channel
    /// left it blank** — KRX blanks this field on some channels
    /// (`V103F` still blanks it), so zero means "not measurable", not
    /// "sequence zero". See `CLAUDE.md`.
    pub sequence: u32,

    /// Exchange board, e.g. `G1`.
    pub board_id: [u8; 2],

    /// Information category, e.g. `04F`.
    pub information_category: [u8; 3],

    /// Applied upper stage.
    pub upper_stage: u8,

    /// Applied lower stage.
    pub lower_stage: u8,

    /// Explicit padding — always zero.
    pub _pad: [u8; 5],
}

const _: () = assert!(size_of::<PriceLimitPayload>() == 40);
const _: () = assert!(
    size_of::<u64>() + 2 * size_of::<BookPrice>() + size_of::<u32>() + 2 + 3 + 1 + 1 + 5
        == size_of::<PriceLimitPayload>()
);

/// Intraday dynamic price band applied or released (40 bytes). Price scale is
/// the header `price_scale`; the instrument is the header ISIN.
///
/// KRX `Q2` / `IFMSRPD0042`. This is the **inner** fence of the order-eligible
/// range and it moves with every print; the outer one is the daily band carried
/// by [`PriceLimitPayload`]. See `documents/krx/실시간가격제한.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct DynamicPriceLimitPayload {
    /// Exchange processing time, ns since KST midnight.
    pub applied_time_of_day: u64,

    /// Dynamic upper price. Meaningless unless `action` is
    /// [`dyn_limit_action::APPLIED`](crate::dyn_limit_action::APPLIED).
    pub upper_price: BookPrice,

    /// Dynamic lower price. Meaningless unless `action` is
    /// [`dyn_limit_action::APPLIED`](crate::dyn_limit_action::APPLIED).
    pub lower_price: BookPrice,

    /// Exchange distribution sequence. **May be zero because the channel left
    /// it blank** — zero means "not measurable", not "sequence zero"
    /// (`CLAUDE.md`).
    pub sequence: u32,

    /// Exchange board, e.g. `G1`.
    pub board_id: [u8; 2],

    /// Information category, e.g. `01F`.
    pub information_category: [u8; 3],

    /// [`dyn_limit_action`](crate::dyn_limit_action) encoding.
    pub action: u8,

    /// Explicit padding — always zero.
    pub _pad: [u8; 6],
}

const _: () = assert!(size_of::<DynamicPriceLimitPayload>() == 40);
const _: () = assert!(
    size_of::<u64>() + 2 * size_of::<BookPrice>() + size_of::<u32>() + 2 + 3 + 1 + 6
        == size_of::<DynamicPriceLimitPayload>()
);

impl DynamicPriceLimitPayload {
    /// `(upper, lower)` if a band is actually in force.
    ///
    /// `None` covers both a release and a code this build does not know, which
    /// is the same instruction to the consumer: do not fence orders with these
    /// numbers.
    #[inline]
    pub const fn band(&self) -> Option<(BookPrice, BookPrice)> {
        if self.action == crate::kind::dyn_limit_action::APPLIED {
            Some((self.upper_price, self.lower_price))
        } else {
            None
        }
    }
}

/// Market operation notice (72 bytes). Scope fields are preserved verbatim
/// so the consumer can decide applicability itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct MarketSchedulePayload {
    /// Event start time, ns since KST midnight.
    pub event_time_of_day: u64,

    /// Scheduled expansion time; meaningful with
    /// [`schedule_flags::EXPECTED_TIME_VALID`](crate::schedule_flags::EXPECTED_TIME_VALID).
    pub expected_time_of_day: u64,

    /// Bit mask selecting affected issue groups; zero for issue actions.
    pub board_event_group: u32,

    /// Information category, e.g. `04F`.
    pub information_category: [u8; 3],

    /// Market-operation product group.
    pub market_operation_product_id: [u8; 3],

    /// Board scope.
    pub board_id: [u8; 2],

    /// Board event identifier (codebook value).
    pub board_event_id: [u8; 3],

    /// `BS`, `BE`, `SS`, `SE`, `SH` or `SR`.
    pub session_action: [u8; 2],

    /// Exchange session identifier.
    pub session_id: [u8; 2],

    /// Product scope.
    pub product_id: [u8; 11],

    /// Common-stock ISIN for cash-market halt scope.
    pub common_stock_isin: [u8; 12],

    /// Cash-market trading-halt reason.
    pub halt_reason: [u8; 3],

    /// Cash-market halt occurrence type.
    pub halt_type: u8,

    /// Applied step of this notice.
    pub step: u8,

    /// [`expansion_direction`](crate::expansion_direction) encoding.
    pub expansion_direction: u8,

    /// [`schedule_flags`](crate::schedule_flags) bits.
    pub schedule_flags: u8,

    /// Explicit padding — always zero.
    pub _pad: [u8; 7],
}

const _: () = assert!(size_of::<MarketSchedulePayload>() == 72);
const _: () = assert!(
    2 * size_of::<u64>() + size_of::<u32>() + 3 + 3 + 2 + 3 + 2 + 2 + 11 + 12 + 3 + 4 + 7
        == size_of::<MarketSchedulePayload>()
);

/// Producer liveness (16 bytes). The header `recv_ns` is the beat time.
///
/// The counters let the consumer tell "network silent" (`received` flat) from
/// "everything filtered" (`received` rising, `forwarded` flat). The beat must
/// be stamped from the receive loop itself — a side thread would keep beating
/// while the receive loop is wedged, which is camouflage, not monitoring (§9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct HeartbeatPayload {
    /// Datagrams / messages received by the handler since boot.
    pub received: u64,

    /// Records written to the ring since boot (excluding heartbeats).
    pub forwarded: u64,
}

const _: () = assert!(size_of::<HeartbeatPayload>() == 16);

// ============================================================================
// Union
// ============================================================================

/// Payload area of a record ([`WIRE_PAYLOAD_LEN`] bytes).
///
/// Which member is meaningful is decided by the header kind; use the typed
/// accessors on [`WireRecord`](crate::WireRecord) rather than reading members
/// directly.
#[derive(Clone, Copy)]
#[repr(C)]
pub union WirePayload {
    /// Raw bytes — always fully initialised.
    pub raw: [u8; WIRE_PAYLOAD_LEN],

    /// [`WireKind::Quote`](crate::WireKind::Quote).
    pub quote: QuotePayload,

    /// [`WireKind::Trade`](crate::WireKind::Trade).
    pub trade: TradePayload,

    /// [`WireKind::TradeQuote`](crate::WireKind::TradeQuote).
    pub trade_quote: TradeQuotePayload,

    /// [`WireKind::OpenInterest`](crate::WireKind::OpenInterest).
    pub open_interest: OpenInterestPayload,

    /// [`WireKind::InvestorStats`](crate::WireKind::InvestorStats).
    pub investor_stats: InvestorStatsPayload,

    /// [`WireKind::PriceLimit`](crate::WireKind::PriceLimit).
    pub price_limit: PriceLimitPayload,

    /// [`WireKind::MarketSchedule`](crate::WireKind::MarketSchedule).
    pub market_schedule: MarketSchedulePayload,

    /// [`WireKind::DynamicPriceLimit`](crate::WireKind::DynamicPriceLimit).
    pub dynamic_price_limit: DynamicPriceLimitPayload,

    /// [`WireKind::Heartbeat`](crate::WireKind::Heartbeat).
    pub heartbeat: HeartbeatPayload,
}

const _: () = assert!(size_of::<WirePayload>() == WIRE_PAYLOAD_LEN);
const _: () = assert!(align_of::<WirePayload>() == 8);
const _: () = assert!(size_of::<TradeQuotePayload>() == WIRE_PAYLOAD_LEN);

impl WirePayload {
    /// All-zero payload.
    pub const ZEROED: Self = Self { raw: [0; WIRE_PAYLOAD_LEN] };

    /// Raw byte view.
    #[inline]
    pub const fn bytes(&self) -> &[u8; WIRE_PAYLOAD_LEN] {
        // SAFETY: `raw` spans the whole union and every member is
        // plain-old-data with explicit padding, so all bytes are initialised.
        unsafe { &self.raw }
    }
}

impl Default for WirePayload {
    #[inline]
    fn default() -> Self {
        Self::ZEROED
    }
}

impl PartialEq for WirePayload {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.bytes() == other.bytes()
    }
}

impl Eq for WirePayload {}

impl fmt::Debug for WirePayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WirePayload({WIRE_PAYLOAD_LEN} bytes)")
    }
}
