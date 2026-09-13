//! `@depth<N>` / `/fapi/v1/depth` — Binance USD-M.
//!
//! USD-M sends a whole book in two different envelopes, and this decoder takes
//! both. The tests are mostly about that: the same book, twice.

use crate::common::*;
use jeed_wire::{WireKind, WireRecord, header_flags, quote_ext};
use jeed_crypto::CryptoError;
use jeed_crypto::binance::futures::snapshot;

#[test]
fn decodes_the_rest_envelope() {
    let inst = futures_btcusdt();
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, FUT_REST_SNAPSHOT, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.depth, 1);
    assert_eq!(rec.validate(), Ok(()));

    let q = rec.quote().unwrap();
    assert_eq!(q.bid[0].price, 400);
    assert_eq!(q.bid[0].qty, 431_000);
    assert_eq!(q.ask[0].price, 401);
    assert_eq!(q.quote_ext_kind, quote_ext::SEQUENCE);
    assert_eq!(q.quote_ext, 1_027_024, "lastUpdateId");
}

#[test]
fn decodes_the_partial_book_stream_envelope() {
    // `@depth10@100ms` says `depthUpdate` and carries `U`/`u`/`pu`, but each
    // frame is a whole book. Routing it to the delta decoder would tell the
    // consumer to *apply* a complete book to its own.
    let inst = futures_btcusdt();
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, FUT_PARTIAL_SNAPSHOT, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Quote), "a snapshot, not a delta");
    let q = rec.quote().unwrap();
    assert_eq!(q.bid[0].price, 740_389);
    assert_eq!(q.bid[0].qty, 2);
    assert_eq!(q.ask[0].price, 740_596);
    assert_eq!(q.ask[0].qty, 3_340);
    assert_eq!(q.quote_ext, 390_497_878, "`u` answers the same question as lastUpdateId");
}

#[test]
fn both_envelopes_prefer_t_over_e() {
    let inst = futures_btcusdt();

    let mut rest = WireRecord::zeroed();
    snapshot::decode(&inst, FUT_REST_SNAPSHOT, RECV_NS, &mut rest).unwrap();
    assert_eq!(rest.header.venue_time(), Some(1_606_292_218_208_000_000));

    let mut stream = WireRecord::zeroed();
    snapshot::decode(&inst, FUT_PARTIAL_SNAPSHOT, RECV_NS, &mut stream).unwrap();
    assert_eq!(stream.header.venue_time(), Some(1_571_889_248_276_000_000));
}

#[test]
fn e_is_used_when_t_is_absent() {
    let inst = futures_btcusdt();
    let frame = br#"{"lastUpdateId":9,"E":1606292218213,"bids":[["4.00","1.000"]],"asks":[["4.01","1.000"]]}"#;
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.venue_time(), Some(1_606_292_218_213_000_000));
}

#[test]
fn a_book_with_no_clock_at_all_is_still_a_book() {
    let inst = futures_btcusdt();
    let frame = br#"{"lastUpdateId":9,"bids":[["4.00","1.000"]],"asks":[["4.01","1.000"]]}"#;
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    assert!(!rec.header.has(header_flags::VENUE_TIME_VALID));
    assert_eq!(rec.quote().unwrap().bid[0].price, 400);
}

#[test]
fn the_stream_envelope_still_checks_the_symbol() {
    let inst = futures_btcusdt();
    let frame = br#"{"e":"depthUpdate","E":1,"T":1,"s":"ETHUSDT","U":1,"u":2,"pu":0,"b":[["4.00","1.000"]],"a":[["4.01","1.000"]]}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        snapshot::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::SymbolMismatch)
    );
    assert_eq!(rec, before);
}
