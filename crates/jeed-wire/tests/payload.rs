//! `jeed_wire::payload` — per-kind payloads and the union.

use jeed_wire::{
    HeartbeatPayload, InvestorStatsPayload, MarketSchedulePayload, OpenInterestPayload,
    PriceLimitPayload, QuotePayload, TradePayload, TradeQuotePayload, WIRE_MAX_DEPTH,
    WIRE_PAYLOAD_LEN, WireLevel, WirePayload, level_ext, quote_ext, trade_flags, trade_kind,
};

#[test]
fn payload_sizes_are_the_documented_ones() {
    assert_eq!(size_of::<WireLevel>(), 24);
    assert_eq!(size_of::<QuotePayload>(), 496);
    assert_eq!(size_of::<TradePayload>(), 32);
    assert_eq!(size_of::<TradeQuotePayload>(), 528);
    assert_eq!(size_of::<OpenInterestPayload>(), 8);
    assert_eq!(size_of::<InvestorStatsPayload>(), 48);
    assert_eq!(size_of::<PriceLimitPayload>(), 40);
    assert_eq!(size_of::<MarketSchedulePayload>(), 72);
    assert_eq!(size_of::<HeartbeatPayload>(), 16);
}

#[test]
fn the_union_is_sized_by_its_largest_member() {
    assert_eq!(size_of::<WirePayload>(), WIRE_PAYLOAD_LEN);
    assert_eq!(size_of::<TradeQuotePayload>(), WIRE_PAYLOAD_LEN);
    assert_eq!(align_of::<WirePayload>(), 8);
}

#[test]
fn a_zeroed_union_reads_as_zero_through_every_member() {
    // This is the property that makes reading any member sound: every byte is
    // initialised and every bit pattern is a valid value of every member.
    let p = WirePayload::ZEROED;
    assert_eq!(p.bytes(), &[0u8; WIRE_PAYLOAD_LEN]);

    // SAFETY: the union is fully initialised and all members are POD.
    unsafe {
        assert_eq!(p.trade, TradePayload::default());
        assert_eq!(p.quote, QuotePayload::default());
        assert_eq!(p.heartbeat, HeartbeatPayload::default());
    }
}

#[test]
fn quote_levels_are_zero_past_depth() {
    let mut q = QuotePayload::default();
    q.set_ask(0, WireLevel::new(1_000, 5));
    q.set_bid(0, WireLevel::new(999, 7));

    assert_eq!(q.asks(1), &[WireLevel::new(1_000, 5)]);
    assert_eq!(q.bids(1), &[WireLevel::new(999, 7)]);
    assert_eq!(q.ask[1], WireLevel::default());
    assert_eq!(q.bid[WIRE_MAX_DEPTH - 1], WireLevel::default());
}

#[test]
fn depth_beyond_the_array_is_clamped_not_panicking() {
    let q = QuotePayload::default();
    assert_eq!(q.asks(u8::MAX).len(), WIRE_MAX_DEPTH);
    assert_eq!(q.bids(u8::MAX).len(), WIRE_MAX_DEPTH);
}

#[test]
fn order_count_validity_is_per_payload_not_per_level() {
    // The channel either carries order counts or it does not; there is no
    // reason for one level to differ from another.
    let mut q = QuotePayload::default();
    assert!(!q.has_order_counts());

    q.set_ask(0, WireLevel::with_count(1_000, 5, 3));
    assert!(!q.has_order_counts(), "the value is there but not yet declared meaningful");

    q.with_order_counts();
    assert!(q.has_order_counts());
    assert_eq!(q.ask[0].order_count, 3);
}

#[test]
fn level_ext_carries_a_bond_yield() {
    let mut q = QuotePayload::default();
    let mut lvl = WireLevel::new(10_000, 1);
    lvl.ext = (-25i32) as u32;
    q.set_ask(0, lvl);
    q.with_level_ext(level_ext::BOND_YIELD);

    assert_eq!(q.level_ext_kind, level_ext::BOND_YIELD);
    assert_eq!(q.ask[0].bond_yield(), -25);
}

#[test]
fn quote_ext_carries_lp_holdings() {
    let mut q = QuotePayload::default();
    q.with_quote_ext(quote_ext::LP_HOLDINGS, 123_456);
    assert_eq!(q.quote_ext_kind, quote_ext::LP_HOLDINGS);
    assert_eq!(q.quote_ext, 123_456);
}

#[test]
fn trade_optional_fields_are_absent_until_marked() {
    let mut t = TradePayload::new(103065, 4);
    assert_eq!(t.trade_kind, trade_kind::NONE);
    assert_eq!(t.cumulative_qty(), None);
    assert_eq!(t.trade_yield(), None);

    t.with_cumulative_qty(166_478);
    assert_eq!(t.cumulative_qty(), Some(166_478));
    assert_eq!(t.trade_yield(), None, "marking one must not mark the other");

    t.with_yield(-3);
    assert_eq!(t.trade_yield(), Some(-3));
    assert_eq!(t.trade_flags, trade_flags::CUMULATIVE_QTY_VALID | trade_flags::YIELD_VALID);
}

#[test]
fn no_aggressor_flag_is_distinct_from_an_unclassified_one() {
    // "this channel has no direction field" and "the field was there but said
    // nothing" are different facts, so they get different values.
    assert_ne!(trade_kind::NONE, trade_kind::UNKNOWN);

    let mut t = TradePayload::new(100, 1);
    assert_eq!(t.trade_kind, trade_kind::NONE);
    t.with_kind(trade_kind::UNKNOWN);
    assert_eq!(t.trade_kind, trade_kind::UNKNOWN);
}

#[test]
fn trade_quote_is_the_trade_and_the_book_it_left() {
    let mut quote = QuotePayload::default();
    quote.set_ask(0, WireLevel::new(93705, 10));
    quote.set_bid(0, WireLevel::new(93695, 8));
    let tq = TradeQuotePayload { trade: TradePayload::new(93700, 2), quote };

    assert_eq!(tq.trade.price, 93700);
    assert_eq!(tq.quote.ask[0].price, 93705);
    assert_eq!(core::mem::offset_of!(TradeQuotePayload, trade), 0);
    assert_eq!(core::mem::offset_of!(TradeQuotePayload, quote), 32);
}
