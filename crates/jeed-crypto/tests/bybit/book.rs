//! `orderbook.{depth}.{symbol}` — Bybit.

use crate::common::*;
use jeed_crypto::CryptoError;
use jeed_crypto::bybit::book;
use jeed_wire::{
    Scale, Venue, WIRE_MAX_DELTA_LEVELS, WireKind, WireRecord, delta_flags, quote_ext,
};

#[test]
fn a_snapshot_becomes_a_whole_book() {
    let inst = linear_btcusdt();
    let mut rec = WireRecord::zeroed();
    book::decode(&inst, ORDERBOOK_SNAPSHOT, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.venue(), Ok(Venue::BybitLinear));
    assert_eq!(rec.header.symbol_bytes(), b"BTCUSDT");
    assert_eq!(rec.header.price_scale(), Ok(Scale::S2));
    assert_eq!(rec.header.qty_scale(), Ok(Scale::S3));
    assert_eq!(rec.header.depth, 2);
    assert_eq!(rec.validate(), Ok(()));

    let q = rec.quote().unwrap();
    assert_eq!(q.bid[0].price, 1_649_350);
    assert_eq!(q.bid[0].qty, 6);
    assert_eq!(q.ask[0].price, 1_661_100);
    assert_eq!(q.ask[1].qty, 213);
    assert_eq!(q.quote_ext_kind, quote_ext::SEQUENCE);
    assert_eq!(q.quote_ext, 18_521_288);
}

#[test]
fn the_matching_engine_clock_wins_over_the_system_clock() {
    // `cts` is when the engine produced the book, `ts` when the system got
    // round to serialising it. Only the first is comparable with a print.
    let inst = linear_btcusdt();
    let mut rec = WireRecord::zeroed();
    book::decode(&inst, ORDERBOOK_SNAPSHOT, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.venue_time(), Some(1_672_304_484_976_000_000));
}

#[test]
fn spot_sends_no_cts_and_falls_back_to_ts() {
    let inst = spot_btcusdt();
    let frame = br#"{"topic":"orderbook.50.BTCUSDT","type":"snapshot","ts":1672304484978,"data":{"s":"BTCUSDT","b":[["16493.50","0.006"]],"a":[["16611.00","0.029"]],"u":18521288,"seq":7961638724}}"#;
    let mut rec = WireRecord::zeroed();
    book::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.venue_time(), Some(1_672_304_484_978_000_000));
    assert_eq!(rec.header.venue(), Ok(Venue::BybitSpot));
}

#[test]
fn one_json_shape_serves_both_categories() {
    // The markets differ by their endpoint and nothing else, which is why they
    // get two venue bytes and one set of decoders.
    let mut linear = WireRecord::zeroed();
    book::decode(&linear_btcusdt(), ORDERBOOK_SNAPSHOT, RECV_NS, &mut linear).unwrap();

    let mut spot = WireRecord::zeroed();
    book::decode(&spot_btcusdt(), ORDERBOOK_SNAPSHOT, RECV_NS, &mut spot).unwrap();

    assert_ne!(linear.header.venue, spot.header.venue);
    linear.header.venue = spot.header.venue;
    assert_eq!(linear, spot);
}

#[test]
fn a_delta_becomes_a_delta() {
    let inst = linear_btcusdt();
    let mut rec = WireRecord::zeroed();
    book::decode(&inst, ORDERBOOK_DELTA, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::SnapshotDelta));
    assert_eq!(rec.validate(), Ok(()));

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.bid_count, 2);
    assert_eq!(d.ask_count, 1);
    assert_eq!(d.bids()[0].price, 1_649_320);
    assert_eq!(d.bids()[0].qty, 30_028);
    assert_eq!(d.bids()[1].qty, 0, "a zero size deletes the price");
    assert_eq!(d.asks()[0].price, 1_661_100);
    assert_eq!(d.asks()[0].qty, 0);
}

#[test]
fn the_chain_is_one_update_id_and_no_predecessor() {
    // Bybit sends no `pu`-equivalent, so the consumer chains on `u + 1`.
    // Leaving the slot absent rather than zero is what lets it tell "this
    // venue does not name its predecessor" from "the predecessor was zero".
    let inst = linear_btcusdt();
    let mut rec = WireRecord::zeroed();
    book::decode(&inst, ORDERBOOK_DELTA, RECV_NS, &mut rec).unwrap();

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.first_update_id, 18_521_289);
    assert_eq!(d.final_update_id, 18_521_289);
    assert_eq!(d.prev_final_update_id(), None);
    assert_eq!(d.delta_flags & delta_flags::PREV_FINAL_VALID, 0);
}

#[test]
fn u_of_one_is_a_rebuild_whatever_the_type_says() {
    // Bybit restarts the orderbook service and resets `u` to 1; the frame may
    // still be typed `delta`. Applying it as one would merge a fresh book into
    // a stale one, so the handler resolves it and the wire carries the
    // conclusion.
    let inst = linear_btcusdt();
    let mut rec = WireRecord::zeroed();
    book::decode(&inst, ORDERBOOK_REBUILD, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.quote().unwrap().quote_ext, 1);
    assert_eq!(rec.header.depth, 1);
}

#[test]
fn a_type_this_build_does_not_know_publishes_nothing() {
    let inst = linear_btcusdt();
    let frame = br#"{"topic":"orderbook.50.BTCUSDT","type":"reset","ts":1672304484978,"data":{"s":"BTCUSDT","b":[],"a":[],"u":5,"seq":1}}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        book::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Unexpected { key: "type" })
    );
    assert_eq!(rec, before);
}

#[test]
fn a_frame_for_another_symbol_is_refused() {
    let inst = linear_btcusdt();
    let frame = br#"{"topic":"orderbook.50.ETHUSDT","type":"snapshot","ts":1672304484978,"data":{"s":"ETHUSDT","b":[["1200.00","1.000"]],"a":[["1201.00","1.000"]],"u":1,"seq":1},"cts":1672304484976}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(book::decode(&inst, frame, RECV_NS, &mut rec), Err(CryptoError::SymbolMismatch));
    assert_eq!(rec, before);
}

#[test]
fn a_frame_with_no_data_object_is_named() {
    let inst = linear_btcusdt();
    let frame = br#"{"topic":"orderbook.50.BTCUSDT","type":"snapshot","ts":1672304484978}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        book::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "data" })
    );
    assert_eq!(rec, before);
}

#[test]
fn a_frame_with_no_update_id_is_named() {
    // `u` decides which payload the frame becomes, so it is read before
    // either can be filled and its absence is fatal in the same place.
    let inst = linear_btcusdt();
    let frame = br#"{"topic":"orderbook.50.BTCUSDT","type":"delta","ts":1672304484978,"data":{"s":"BTCUSDT","b":[["16493.50","0.006"]],"a":[],"seq":1}}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        book::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "u" })
    );
    assert_eq!(rec, before);
}

#[test]
fn a_diff_deeper_than_the_wire_is_dropped_whole() {
    let inst = linear_btcusdt();
    let mut frame =
        br#"{"topic":"orderbook.500.BTCUSDT","type":"delta","ts":1672304484978,"data":{"s":"BTCUSDT","b":["#
            .to_vec();
    for i in 0..(WIRE_MAX_DELTA_LEVELS as u64 + 1) {
        if i > 0 {
            frame.push(b',');
        }
        frame.extend_from_slice(format!(r#"["{}.00","1.000"]"#, 16_000 + i).as_bytes());
    }
    frame.extend_from_slice(br#"],"a":[],"u":18521290,"seq":1}}"#);

    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        book::decode(&inst, &frame, RECV_NS, &mut rec),
        Err(CryptoError::DeltaOverflow { max: WIRE_MAX_DELTA_LEVELS })
    );
    assert_eq!(rec, before);
}
