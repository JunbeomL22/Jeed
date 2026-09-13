//! `/api/v4/spot/order_book` — Gate.

use crate::common::*;
use jeed_crypto::CryptoError;
use jeed_crypto::gate::snapshot;
use jeed_wire::{Venue, WireKind, WireRecord, quote_ext};

#[test]
fn a_rest_body_becomes_a_whole_book() {
    let inst = btc_usdt();
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, REST_ORDER_BOOK, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.venue(), Ok(Venue::GateSpot));
    assert_eq!(rec.header.depth, 2);
    assert_eq!(rec.header.venue_time(), Some(1_606_295_412_100_000_000), "`update`, not `current`");
    assert_eq!(rec.validate(), Ok(()));

    let q = rec.quote().unwrap();
    assert_eq!(q.ask[0].price, 1_913_775);
    assert_eq!(q.ask[1].price, 1_913_800);
    assert_eq!(q.bid[0].price, 1_913_774);
}

#[test]
fn the_book_id_is_carried_because_the_deltas_chain_onto_it() {
    // `id` and the WS channel's `u` are the same number space, which is the
    // only reason a resynchronisation can work.
    let inst = btc_usdt();
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, REST_ORDER_BOOK, RECV_NS, &mut rec).unwrap();

    let q = rec.quote().unwrap();
    assert_eq!(q.quote_ext_kind, quote_ext::SEQUENCE);
    assert_eq!(q.quote_ext, 48_776_300);
}

#[test]
fn a_body_fetched_without_an_id_is_still_a_book() {
    // `with_id=true` is the caller's to remember. Without it the book is a
    // picture rather than a starting point, and the absent extension says so.
    let inst = btc_usdt();
    let frame = br#"{"current":1606295412123,"update":1606295412100,"asks":[["19137.75","0.6135"]],"bids":[["19137.74","0.0001"]]}"#;

    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.quote().unwrap().quote_ext_kind, quote_ext::NONE);
}

#[test]
fn a_body_with_no_sides_is_refused() {
    let inst = btc_usdt();
    let frame = br#"{"label":"INVALID_CURRENCY_PAIR","message":"Invalid currency pair BTC_USDTT"}"#;

    let mut rec = dirty_record();
    let before = rec;
    assert!(matches!(
        snapshot::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { .. })
    ));
    assert_eq!(rec, before);
}
