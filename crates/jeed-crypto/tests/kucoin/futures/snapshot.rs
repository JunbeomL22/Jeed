//! `/api/v1/level2/snapshot` — KuCoin futures.

use crate::common::*;
use jeed_crypto::CryptoError;
use jeed_crypto::kucoin::futures::snapshot;
use jeed_wire::{Venue, WireKind, WireRecord, quote_ext};

#[test]
fn a_quoted_price_beside_a_bare_size_reads_as_one_level() {
    let inst = futures_xbtusdtm();
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, FUTURES_REST, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.venue(), Ok(Venue::KucoinFutures));
    assert_eq!(rec.header.depth, 2);
    assert_eq!(rec.validate(), Ok(()));

    let q = rec.quote().unwrap();
    assert_eq!(q.ask[0].price, 906_312);
    assert_eq!(q.ask[0].qty, 2);
    assert_eq!(q.ask[1].price, 906_320);
    assert_eq!(q.ask[1].qty, 7);
    assert_eq!(q.bid[0].price, 906_308);
    assert_eq!(q.bid[0].qty, 5);
}

#[test]
fn the_sequence_is_carried() {
    let inst = futures_xbtusdtm();
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, FUTURES_REST, RECV_NS, &mut rec).unwrap();

    let q = rec.quote().unwrap();
    assert_eq!(q.quote_ext_kind, quote_ext::SEQUENCE);
    assert_eq!(q.quote_ext, 1_709_400_450_243);
}

#[test]
fn no_clock_is_claimed_for_it() {
    // Nothing in this body has a unit this port has verified, and on KuCoin
    // the guess is between milliseconds and nanoseconds.
    let inst = futures_xbtusdtm();
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, FUTURES_REST, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.venue_time(), None);
    assert_eq!(rec.header.recv_ns, RECV_NS);
}

#[test]
fn a_body_for_another_instrument_is_refused() {
    let inst = futures_xbtusdtm();
    let frame = br#"{"code":"200000","data":{"symbol":"ETHUSDTM","sequence":1,"asks":[["3000.0",1]],"bids":[["2999.0",1]]}}"#;

    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(snapshot::decode(&inst, frame, RECV_NS, &mut rec), Err(CryptoError::SymbolMismatch));
    assert_eq!(rec, before);
}

#[test]
fn a_body_with_no_book_in_it_is_refused() {
    let inst = futures_xbtusdtm();
    let frame = br#"{"code":"404000","msg":"Not Found"}"#;

    let mut rec = WireRecord::zeroed();
    assert_eq!(
        snapshot::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "data" })
    );
}
