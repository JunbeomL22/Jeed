//! REST depth — Bitget.

use crate::common::*;
use jeed_crypto::CryptoError;
use jeed_crypto::bitget::snapshot;
use jeed_wire::{Venue, WireKind, WireRecord, quote_ext};

#[test]
fn a_rest_body_becomes_a_whole_book() {
    let inst = linear_btcusdt();
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, REST_DEPTH, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.venue(), Ok(Venue::BitgetLinear));
    assert_eq!(rec.header.depth, 2, "the deeper side decides the header's depth");
    assert_eq!(rec.header.venue_time(), Some(1_706_000_000_000_000_000));
    assert_eq!(rec.validate(), Ok(()));

    let q = rec.quote().unwrap();
    assert_eq!(q.ask[0].price, 270_010);
    assert_eq!(q.ask[1].price, 270_020);
    assert_eq!(q.bid[0].price, 270_000);
    assert_eq!(q.bid[0].qty, 2_300);
}

#[test]
fn no_sequence_is_invented_for_it() {
    // The REST book and the WS `seq` are not in the same number space, so
    // there is nothing to chain and the extension stays absent.
    let inst = linear_btcusdt();
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, REST_DEPTH, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.quote().unwrap().quote_ext_kind, quote_ext::NONE);
    assert_eq!(rec.quote().unwrap().quote_ext, 0);
}

#[test]
fn a_body_with_no_book_in_it_is_refused() {
    // A rejected request answers with `code` and `msg` and no `data`; reading
    // what the message has says so without a table of Bitget's error codes.
    let inst = linear_btcusdt();
    let frame = br#"{"code":"40034","msg":"Parameter symbol does not exist","requestTime":1706000000000,"data":null}"#;

    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(
        snapshot::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "data" })
    );
    assert_eq!(rec, before);
}

#[test]
fn a_book_with_only_one_side_is_still_a_book() {
    let inst = linear_btcusdt();
    let frame = br#"{"code":"00000","data":{"asks":[["27001.0","1.200"]],"bids":[],"ts":"1706000000000"}}"#;

    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    let q = rec.quote().unwrap();
    assert_eq!(q.ask[0].price, 270_010);
    assert_eq!(q.bid[0], Default::default());
}
