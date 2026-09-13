//! `@aggTrade` — Binance USD-M.

use crate::common::*;
use jeed_wire::{Venue, WireKind, WireRecord, header_flags, trade_kind};
use jeed_crypto::binance::futures::trade;

#[test]
fn decodes_the_documented_frame() {
    let inst = futures_btcusdt();
    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, FUT_AGG_TRADE, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Trade));
    assert_eq!(rec.header.venue(), Ok(Venue::BinanceFutures));
    assert_eq!(rec.validate(), Ok(()));

    let t = rec.trade().unwrap();
    assert_eq!(t.price, 740_389);
    assert_eq!(t.qty, 100_000, "100.000 at three decimals");
    assert_eq!(t.trade_kind, trade_kind::BUY, "m=false — the buyer took");

    assert!(rec.header.has(header_flags::VENUE_TIME_VALID));
    assert_eq!(rec.header.venue_time(), Some(1_672_515_782_136_000_000));
}

#[test]
fn the_aggregate_size_is_not_a_cumulative_size() {
    // `q` is this taker order's total fill, not the session's. Putting it in
    // `cumulative_qty` would give the consumer a number that means something
    // else on every other feed.
    let inst = futures_btcusdt();
    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, FUT_AGG_TRADE, RECV_NS, &mut rec).unwrap();
    assert_eq!(rec.trade().unwrap().cumulative_qty(), None);
}

#[test]
fn the_first_and_last_trade_ids_are_skipped_without_confusing_the_scan() {
    // `f` and `l` sit between `q` and `T` in the frame. A scanner that lost
    // its place on them would read `T` as something else.
    let inst = futures_btcusdt();
    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, FUT_AGG_TRADE, RECV_NS, &mut rec).unwrap();
    assert_eq!(rec.header.venue_time(), Some(1_672_515_782_136_000_000));
}
