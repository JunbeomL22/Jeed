//! `@depth<N>` / `/api/v3/depth` — Binance spot.

use crate::common::*;
use jeed_crypto::CryptoError;
use jeed_crypto::binance::spot::snapshot;
use jeed_wire::{WIRE_MAX_DEPTH, WireKind, WireRecord, header_flags, quote_ext};

#[test]
fn decodes_the_documented_frame() {
    let inst = spot_btcusdt();
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, SPOT_SNAPSHOT, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.validate(), Ok(()));
    assert_eq!(rec.header.depth, 3, "the deeper side sets the depth");

    let q = rec.quote().unwrap();
    assert_eq!(q.bid[0].price, 400);
    assert_eq!(q.bid[0].qty, 43_100_000);
    assert_eq!(q.bid[1].price, 399);
    assert_eq!(q.bid[2].price, 398);
    assert_eq!(q.ask[0].price, 401);
    assert_eq!(q.ask[1].price, 402);
    assert_eq!(q.ask[2], Default::default(), "past the ask side's depth");
}

#[test]
fn last_update_id_rides_as_the_sequence() {
    // It is the same counter the diff stream chains on, which is what makes a
    // snapshot usable to recover from a gap.
    let inst = spot_btcusdt();
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, SPOT_SNAPSHOT, RECV_NS, &mut rec).unwrap();

    let q = rec.quote().unwrap();
    assert_eq!(q.quote_ext_kind, quote_ext::SEQUENCE);
    assert_eq!(q.quote_ext, 1_027_024);
}

#[test]
fn depth_beyond_the_wire_is_dropped_not_refused() {
    // REST sends up to 5,000 levels and the wire carries ten. Truncating a
    // snapshot is a shallower book, which is a book; truncating a delta would
    // be a wrong book, which is why only this side truncates.
    let inst = spot_btcusdt();
    let mut levels = String::new();
    for i in 0..40 {
        if i > 0 {
            levels.push(',');
        }
        levels.push_str(&format!(r#"["{}.00000000","1.00000000"]"#, 1000 - i));
    }
    let frame = format!(r#"{{"lastUpdateId":9,"bids":[{levels}],"asks":[["2000.00000000","1.00000000"]]}}"#);

    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, frame.as_bytes(), RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.depth as usize, WIRE_MAX_DEPTH);
    let q = rec.quote().unwrap();
    assert_eq!(q.bid[0].price, 100_000);
    assert_eq!(q.bid[9].price, 99_100, "the tenth level, then the array is skipped");
    assert_eq!(q.ask[0].price, 200_000, "the asks after it still parse");
}

#[test]
fn an_empty_side_is_flagged() {
    let inst = spot_btcusdt();
    let frame = br#"{"lastUpdateId":9,"bids":[],"asks":[["4.01000000","12.00000000"]]}"#;
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    assert!(rec.header.has(header_flags::BID_EMPTY));
    assert!(!rec.header.has(header_flags::ASK_EMPTY));
    assert_eq!(rec.header.depth, 1);
}

#[test]
fn depth_counts_quantity_not_levels() {
    // A level that arrived with no size is not depth — the same rule
    // `jeed_krx::decode::common::BookAccum` applies to a zero-padded KRX book.
    let inst = spot_btcusdt();
    let frame = br#"{"lastUpdateId":9,"bids":[["4.00000000","1.00000000"],["3.99000000","0"]],"asks":[["4.01000000","1.00000000"]]}"#;
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.depth, 1);
    assert_eq!(rec.quote().unwrap().bid[1].price, 399, "still carried, just not counted");
}

#[test]
fn a_snapshot_without_a_sequence_is_refused() {
    // Without it the record cannot be used to recover, which is the only
    // reason to publish a snapshot on a delta feed.
    let inst = spot_btcusdt();
    let frame = br#"{"bids":[["4.00000000","1.00000000"]],"asks":[["4.01000000","1.00000000"]]}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        snapshot::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "lastUpdateId" })
    );
    assert_eq!(rec, before);
}

#[test]
fn a_bad_level_leaves_the_record_alone() {
    let inst = spot_btcusdt();
    let frame = br#"{"lastUpdateId":9,"bids":[["4.00000000","nope"]],"asks":[]}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert!(snapshot::decode(&inst, frame, RECV_NS, &mut rec).is_err());
    assert_eq!(rec, before, "half a book must never reach the ring");
}
