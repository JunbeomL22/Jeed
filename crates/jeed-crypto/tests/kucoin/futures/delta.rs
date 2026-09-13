//! `/contractMarket/level2` — KuCoin futures.

use crate::common::*;
use jeed_crypto::CryptoError;
use jeed_crypto::kucoin::futures::delta;
use jeed_wire::{Scale, Venue, WireKind, WireRecord};

#[test]
fn one_change_becomes_one_level_on_one_side() {
    let inst = futures_xbtusdtm();
    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, FUTURES_LEVEL2, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::SnapshotDelta));
    assert_eq!(rec.header.venue(), Ok(Venue::KucoinFutures));
    assert_eq!(rec.header.symbol_bytes(), b"XBTUSDTM");
    assert_eq!(rec.header.price_scale(), Ok(Scale::S1));
    assert_eq!(rec.header.qty_scale(), Ok(Scale::S0), "sizes are whole contracts");
    assert_eq!(rec.validate(), Ok(()));

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.bid_count, 0);
    assert_eq!(d.ask_count, 1, "`sell` is the ask side");
    assert_eq!(d.asks()[0].price, 906_312);
    assert_eq!(d.asks()[0].qty, 2);
    assert_eq!(d.first_update_id, 1_709_400_450_243);
    assert_eq!(d.final_update_id, 1_709_400_450_243);
    assert_eq!(rec.header.venue_time(), Some(1_731_897_467_182_000_000));
}

#[test]
fn a_buy_change_lands_on_the_bid_side() {
    let inst = futures_xbtusdtm();
    let frame = br#"{"topic":"/contractMarket/level2:XBTUSDTM","type":"message","subject":"level2","data":{"sequence":1709400450244,"change":"90630.8,buy,5","timestamp":1731897467183}}"#;

    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.bid_count, 1);
    assert_eq!(d.ask_count, 0);
    assert_eq!(d.bids()[0].price, 906_308);
    assert_eq!(d.bids()[0].qty, 5);
}

#[test]
fn a_zero_size_deletes_the_price() {
    let inst = futures_xbtusdtm();
    let frame = br#"{"topic":"/contractMarket/level2:XBTUSDTM","type":"message","subject":"level2","data":{"sequence":1,"change":"90631.2,sell,0","timestamp":1}}"#;

    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.snapshot_delta().unwrap().asks()[0].qty, 0);
}

#[test]
fn a_change_with_a_side_this_build_cannot_read_publishes_nothing() {
    // Putting a change on the wrong side leaves two prices wrong and no later
    // message corrects either.
    let inst = futures_xbtusdtm();
    let frame = br#"{"topic":"/contractMarket/level2:XBTUSDTM","type":"message","subject":"level2","data":{"sequence":1,"change":"90631.2,auction,2","timestamp":1}}"#;

    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(
        delta::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Unexpected { key: "change" })
    );
    assert_eq!(rec, before);
}

#[test]
fn a_change_that_is_not_three_fields_publishes_nothing() {
    let inst = futures_xbtusdtm();
    let frame = br#"{"topic":"/contractMarket/level2:XBTUSDTM","type":"message","subject":"level2","data":{"sequence":1,"change":"90631.2,sell","timestamp":1}}"#;

    let mut rec = WireRecord::zeroed();
    assert_eq!(
        delta::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Unexpected { key: "change" })
    );
}

#[test]
fn the_topic_is_the_only_identity_and_it_is_checked() {
    // A futures book message carries no `symbol` field at all.
    let inst = futures_xbtusdtm();
    let frame = br#"{"topic":"/contractMarket/level2:ETHUSDTM","type":"message","subject":"level2","data":{"sequence":1,"change":"3000.0,buy,1","timestamp":1}}"#;

    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(delta::decode(&inst, frame, RECV_NS, &mut rec), Err(CryptoError::SymbolMismatch));
    assert_eq!(rec, before);
}

#[test]
fn a_topic_naming_no_symbol_is_refused_rather_than_ignored() {
    let inst = futures_xbtusdtm();
    let frame = br#"{"topic":"/contractMarket/level2","type":"message","subject":"level2","data":{"sequence":1,"change":"90631.2,buy,1","timestamp":1}}"#;

    let mut rec = WireRecord::zeroed();
    assert_eq!(delta::decode(&inst, frame, RECV_NS, &mut rec), Err(CryptoError::SymbolMismatch));
}

#[test]
fn a_diff_with_no_sequence_is_refused() {
    let inst = futures_xbtusdtm();
    let frame = br#"{"topic":"/contractMarket/level2:XBTUSDTM","type":"message","subject":"level2","data":{"change":"90631.2,buy,1","timestamp":1}}"#;

    let mut rec = WireRecord::zeroed();
    assert_eq!(
        delta::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "sequence" })
    );
}
