//! `/api/v3/market/orderbook/level2` — KuCoin spot.

use crate::common::*;
use jeed_crypto::CryptoError;
use jeed_crypto::kucoin::spot::snapshot;
use jeed_wire::{Venue, WireKind, WireRecord, quote_ext};

#[test]
fn a_rest_body_becomes_a_whole_book() {
    let inst = spot_btc_usdt();
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, SPOT_REST, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.venue(), Ok(Venue::KucoinSpot));
    assert_eq!(rec.header.depth, 2);
    assert_eq!(rec.validate(), Ok(()));

    let q = rec.quote().unwrap();
    assert_eq!(q.bid[0].price, 35_355);
    assert_eq!(q.bid[1].price, 35_350);
    assert_eq!(q.ask[0].price, 35_361);
    assert_eq!(q.ask[0].qty, 10_000_000);
}

#[test]
fn the_sequence_is_carried_because_the_diffs_chain_onto_it() {
    let inst = spot_btc_usdt();
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, SPOT_REST, RECV_NS, &mut rec).unwrap();

    let q = rec.quote().unwrap();
    assert_eq!(q.quote_ext_kind, quote_ext::SEQUENCE);
    assert_eq!(q.quote_ext, 1_602_997_267_139);
}

#[test]
fn this_time_is_milliseconds_where_the_trade_channels_is_nanoseconds() {
    // The same key name, six orders of magnitude apart, on two endpoints of
    // the same venue.
    let inst = spot_btc_usdt();
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, SPOT_REST, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.venue_time(), Some(1_602_997_267_139_000_000));
}

#[test]
fn a_body_with_no_book_in_it_is_refused() {
    let inst = spot_btc_usdt();
    let frame = br#"{"code":"400100","msg":"Invalid symbol"}"#;

    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(
        snapshot::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "data" })
    );
    assert_eq!(rec, before);
}
