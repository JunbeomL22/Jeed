//! `spot.trades` — Gate.

use crate::common::*;
use jeed_crypto::CryptoError;
use jeed_crypto::gate::trade;
use jeed_wire::{Venue, WireKind, WireRecord, trade_kind};

#[test]
fn a_print_carries_price_size_and_the_takers_side() {
    let inst = btc_usdt();
    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, TRADE, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Trade));
    assert_eq!(rec.header.venue(), Ok(Venue::GateSpot));
    assert_eq!(rec.header.symbol_bytes(), b"BTC_USDT");
    assert_eq!(rec.validate(), Ok(()));

    let t = rec.trade().unwrap();
    assert_eq!(t.price, 1_913_775);
    assert_eq!(t.qty, 164_700);
    assert_eq!(t.trade_kind, trade_kind::SELL);
}

#[test]
fn the_sub_millisecond_digits_survive() {
    // `"1606292218213.4578"` is 1606292218213 ms and 457.8 µs more. Gate
    // measured those digits; truncating at the point throws them away.
    let inst = btc_usdt();
    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, TRADE, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.venue_time(), Some(1_606_292_218_213_457_800));
}

#[test]
fn a_frame_for_another_instrument_is_refused() {
    let inst = btc_usdt();
    let frame = br#"{"channel":"spot.trades","event":"update","result":{"id":1,"create_time_ms":"1606292218213.0","side":"buy","currency_pair":"GT_USDT","amount":"1.0000","price":"0.4705"}}"#;

    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(trade::decode(&inst, frame, RECV_NS, &mut rec), Err(CryptoError::SymbolMismatch));
    assert_eq!(rec, before);
}

#[test]
fn a_print_with_no_amount_leaves_the_record_alone() {
    let inst = btc_usdt();
    let frame = br#"{"channel":"spot.trades","event":"update","result":{"id":1,"create_time_ms":"1606292218213.0","side":"buy","currency_pair":"BTC_USDT","price":"19137.75"}}"#;

    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(
        trade::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "amount" })
    );
    assert_eq!(rec, before);
}

#[test]
fn a_side_this_build_does_not_know_still_publishes_the_print() {
    let inst = btc_usdt();
    let frame = br#"{"channel":"spot.trades","event":"update","result":{"id":1,"create_time_ms":"1606292218213.0","side":"","currency_pair":"BTC_USDT","amount":"1.0000","price":"19137.75"}}"#;

    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.trade().unwrap().trade_kind, trade_kind::UNKNOWN);
    assert_eq!(rec.trade().unwrap().price, 1_913_775);
}

#[test]
fn a_timestamp_that_is_not_a_number_leaves_the_clock_unset() {
    // The print is still a print; what is missing is only when it happened,
    // and the header says so rather than guessing.
    let inst = btc_usdt();
    let frame = br#"{"channel":"spot.trades","event":"update","result":{"id":1,"create_time_ms":"","side":"buy","currency_pair":"BTC_USDT","amount":"1.0000","price":"19137.75"}}"#;

    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.venue_time(), None);
    assert_eq!(rec.trade().unwrap().qty, 10_000);
}
