//! `jeed_wire::payload` — per-kind payloads and the union.

use jeed_wire::{
    HeartbeatPayload, InvestorStatsPayload, MarketSchedulePayload, OpenInterestPayload,
    PriceLimitPayload, QuotePayload, SnapshotDeltaPayload, TradePayload, TradeQuotePayload,
    WIRE_MAX_DELTA_LEVELS, WIRE_MAX_DEPTH, WIRE_PAYLOAD_LEN, WireDeltaLevel, WireLevel,
    WirePayload, delta_flags, level_ext, quote_ext, trade_flags, trade_kind,
};

#[test]
fn payload_sizes_are_the_documented_ones() {
    assert_eq!(size_of::<WireLevel>(), 24);
    assert_eq!(size_of::<QuotePayload>(), 496);
    assert_eq!(size_of::<TradePayload>(), 48);
    assert_eq!(size_of::<TradeQuotePayload>(), 544);
    assert_eq!(size_of::<OpenInterestPayload>(), 8);
    assert_eq!(size_of::<InvestorStatsPayload>(), 48);
    assert_eq!(size_of::<PriceLimitPayload>(), 40);
    assert_eq!(size_of::<MarketSchedulePayload>(), 72);
    assert_eq!(size_of::<HeartbeatPayload>(), 16);
    assert_eq!(size_of::<WireDeltaLevel>(), 16);
    assert_eq!(size_of::<SnapshotDeltaPayload>(), 544);
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
fn dynamic_price_limits_are_absent_until_marked() {
    // KRX sends `000000.00`, not blanks, for instruments outside the dynamic
    // limit regime — so a zero band and an absent one look identical in the
    // raw message and the conclusion has to ride in a flag.
    let mut t = TradePayload::new(93700, 2);
    assert_eq!(t.dyn_limits(), None);
    assert_eq!(t.dyn_upper, 0);

    t.with_dyn_limits(0, 0);
    assert_eq!(t.dyn_limits(), Some((0, 0)), "a zero band that is really in force");
    assert!(t.trade_flags & trade_flags::DYN_LIMIT_VALID != 0);
}

#[test]
fn the_three_optional_trade_fields_are_marked_independently() {
    let mut t = TradePayload::new(93700, 2);
    t.with_dyn_limits(93_700 + 900, 93_700 - 900);

    assert_eq!(t.dyn_limits(), Some((94_600, 92_800)));
    assert_eq!(t.cumulative_qty(), None, "marking one must not mark the others");
    assert_eq!(t.trade_yield(), None);
    assert_eq!(t.trade_flags, trade_flags::DYN_LIMIT_VALID);
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
    let mut trade = TradePayload::new(93700, 2);
    // The inner fence of the order-eligible range, from the same G7 message
    // that carried the print (documents/krx/실시간가격제한.md).
    trade.with_dyn_limits(93_700 + 865, 93_700 - 865);
    let tq = TradeQuotePayload { trade, quote };

    assert_eq!(tq.trade.dyn_limits(), Some((94_565, 92_835)));
    assert_eq!(tq.trade.price, 93700);
    assert_eq!(tq.quote.ask[0].price, 93705);
    assert_eq!(core::mem::offset_of!(TradeQuotePayload, trade), 0);
    assert_eq!(core::mem::offset_of!(TradeQuotePayload, quote), 48);
}

// ============================================================================
// Snapshot delta
// ============================================================================

/// A delta with `bids` bid changes and `asks` ask changes, prices numbered so
/// a misread of the split shows up as a wrong price rather than a wrong count.
fn delta(bids: u8, asks: u8) -> SnapshotDeltaPayload {
    let mut d = SnapshotDeltaPayload::default();
    for i in 0..bids as usize {
        d.levels[i] = WireDeltaLevel { price: 1_000 + i as i64, qty: 10 };
    }
    for i in 0..asks as usize {
        d.levels[bids as usize + i] = WireDeltaLevel { price: 2_000 + i as i64, qty: 20 };
    }
    d.bid_count = bids;
    d.ask_count = asks;
    d
}

#[test]
fn delta_fills_the_payload_area_exactly() {
    // Sized to land on WIRE_PAYLOAD_LEN rather than to push it up: 32 levels
    // at 16 bytes is 512, and the ids and counts are the remaining 32.
    assert_eq!(
        size_of::<WireDeltaLevel>() * WIRE_MAX_DELTA_LEVELS + 32,
        WIRE_PAYLOAD_LEN
    );
    assert_eq!(size_of::<SnapshotDeltaPayload>(), WIRE_PAYLOAD_LEN);
}

#[test]
fn the_two_sides_share_one_array() {
    // The point of sharing: a lopsided message fits where a split array of 16
    // and 16 would have refused it.
    let d = delta(30, 2);
    assert_eq!(d.bids().len(), 30);
    assert_eq!(d.asks().len(), 2);
    assert_eq!(d.level_count(), 32);
    assert_eq!(d.bids()[0].price, 1_000);
    assert_eq!(d.asks()[0].price, 2_000, "asks start where the bids stop");
    assert_eq!(d.asks()[1].price, 2_001);
}

#[test]
fn one_sided_delta_reads_as_one_sided() {
    let d = delta(0, 3);
    assert!(d.bids().is_empty());
    assert_eq!(d.asks().len(), 3);
    assert_eq!(d.asks()[0].price, 2_000);
}

#[test]
fn accessors_stay_inside_the_array_when_counts_lie() {
    // A record off the wire can claim anything; validate() rejects it, but the
    // accessors must not panic on the way to finding that out.
    let mut d = delta(2, 2);
    d.bid_count = 200;
    d.ask_count = 200;
    assert_eq!(d.bids().len(), WIRE_MAX_DELTA_LEVELS);
    assert!(d.asks().is_empty(), "no room left after the bids took it all");
}

#[test]
fn zero_quantity_is_a_deletion_and_survives_the_round_trip() {
    // The venue's own encoding for "this price is gone". It must not be
    // mistaken for an unused array entry, which is why the count is authority.
    let mut d = delta(1, 0);
    d.levels[0].qty = 0;
    assert_eq!(d.bids().len(), 1);
    assert_eq!(d.bids()[0], WireDeltaLevel { price: 1_000, qty: 0 });
}

#[test]
fn prev_final_update_id_is_absent_until_a_venue_sends_one() {
    let mut d = delta(1, 1);
    assert_eq!(d.prev_final_update_id(), None, "Binance spot sends no `pu`");

    // Zero must read as a real id once set — that is why the flag exists.
    d.with_prev_final_update_id(0);
    assert_eq!(d.prev_final_update_id(), Some(0));
    assert!(d.delta_flags & delta_flags::PREV_FINAL_VALID != 0);
}
