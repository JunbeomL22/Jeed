//! `@depth` — Binance spot incremental updates.

use crate::common::*;
use jeed_crypto::CryptoError;
use jeed_crypto::binance::spot::delta;
use jeed_wire::{WIRE_MAX_DELTA_LEVELS, WireDeltaLevel, WireKind, WireRecord, header_flags};

#[test]
fn decodes_the_documented_frame() {
    let inst = spot_btcusdt();
    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, SPOT_DELTA, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::SnapshotDelta));
    assert_eq!(rec.header.symbol_bytes(), b"BTCUSDT");
    assert_eq!(rec.validate(), Ok(()));

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.first_update_id, 157);
    assert_eq!(d.final_update_id, 160);
    assert_eq!(d.bids(), &[WireDeltaLevel { price: 11_925_000, qty: 1_000_000 }]);
    assert_eq!(d.asks(), &[WireDeltaLevel { price: 11_926_000, qty: 0 }]);
}

#[test]
fn a_zero_quantity_is_a_deletion_and_survives() {
    // The venue's own way of saying "this price is gone". Dropping the level
    // because its size is zero would leave the price resting forever.
    let inst = spot_btcusdt();
    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, SPOT_DELTA, RECV_NS, &mut rec).unwrap();

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.ask_count, 1);
    assert_eq!(d.asks()[0].qty, 0);
}

#[test]
fn spot_sends_no_pu_so_the_field_stays_absent() {
    // USD-M names the message a delta must follow; spot does not, and a zero
    // must not be read as "follows message zero".
    let inst = spot_btcusdt();
    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, SPOT_DELTA, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.snapshot_delta().unwrap().prev_final_update_id(), None);
}

#[test]
fn venue_time_comes_from_e_because_spot_has_no_t() {
    let inst = spot_btcusdt();
    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, SPOT_DELTA, RECV_NS, &mut rec).unwrap();

    assert!(rec.header.has(header_flags::VENUE_TIME_VALID));
    assert_eq!(rec.header.venue_time(), Some(1_755_088_771_745_000_000));
}

#[test]
fn a_delta_carries_no_header_depth() {
    // `depth` is levels per side of a book. A delta has neither a book nor
    // sides of equal length; the counts are in the payload.
    let inst = spot_btcusdt();
    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, SPOT_DELTA, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.depth, 0);
    let d = rec.snapshot_delta().unwrap();
    assert_eq!((d.bid_count, d.ask_count), (1, 1));
}

#[test]
fn the_two_sides_share_the_array_without_a_split_down_the_middle() {
    // Twenty bid changes and two ask changes is an ordinary message. A payload
    // of sixteen per side would have refused it.
    let inst = spot_btcusdt();
    let bids: Vec<String> =
        (0..20).map(|i| format!(r#"["{}.00000000","1.00000000"]"#, 1000 - i)).collect();
    let frame = format!(
        r#"{{"e":"depthUpdate","E":1,"s":"BTCUSDT","U":1,"u":2,"b":[{}],"a":[["2000.00000000","1.00000000"],["2001.00000000","0"]]}}"#,
        bids.join(",")
    );

    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, frame.as_bytes(), RECV_NS, &mut rec).unwrap();

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.bid_count, 20);
    assert_eq!(d.ask_count, 2);
    assert_eq!(d.bids()[19].price, 98_100);
    assert_eq!(d.asks()[0].price, 200_000);
    assert_eq!(rec.validate(), Ok(()));
}

#[test]
fn asks_before_bids_still_land_bids_first() {
    // Binance sends `b` then `a`, but key order in a JSON object is not
    // something a decoder gets to assume, and the wire lays bids out first.
    let inst = spot_btcusdt();
    let frame = br#"{"e":"depthUpdate","E":1,"s":"BTCUSDT","U":1,"u":2,"a":[["2000.00000000","5.00000000"],["2001.00000000","6.00000000"]],"b":[["1000.00000000","1.00000000"]]}"#;

    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.bids(), &[WireDeltaLevel { price: 100_000, qty: 100_000 }]);
    assert_eq!(
        d.asks(),
        &[
            WireDeltaLevel { price: 200_000, qty: 500_000 },
            WireDeltaLevel { price: 200_100, qty: 600_000 },
        ]
    );
}

#[test]
fn more_changes_than_the_record_holds_drops_the_frame() {
    // Not truncated. A consumer that accepted thirty-two of forty changes
    // would never be told to re-apply the other eight; a consumer that meets
    // the resulting hole in the update-id chain resynchronises, which is the
    // machinery it already needs for a ring drop.
    let inst = spot_btcusdt();
    let bids: Vec<String> =
        (0..40).map(|i| format!(r#"["{}.00000000","1.00000000"]"#, 1000 - i)).collect();
    let frame = format!(
        r#"{{"e":"depthUpdate","E":1,"s":"BTCUSDT","U":1,"u":2,"b":[{}],"a":[]}}"#,
        bids.join(",")
    );

    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(
        delta::decode(&inst, frame.as_bytes(), RECV_NS, &mut rec),
        Err(CryptoError::DeltaOverflow { max: WIRE_MAX_DELTA_LEVELS })
    );
    assert_eq!(rec, before, "nothing is published, so nothing is half-applied");
}

#[test]
fn exactly_the_capacity_fits() {
    let inst = spot_btcusdt();
    let bids: Vec<String> = (0..WIRE_MAX_DELTA_LEVELS)
        .map(|i| format!(r#"["{}.00000000","1.00000000"]"#, 1000 - i))
        .collect();
    let frame = format!(
        r#"{{"e":"depthUpdate","E":1,"s":"BTCUSDT","U":1,"u":2,"b":[{}],"a":[]}}"#,
        bids.join(",")
    );

    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, frame.as_bytes(), RECV_NS, &mut rec).unwrap();
    assert_eq!(rec.snapshot_delta().unwrap().bid_count as usize, WIRE_MAX_DELTA_LEVELS);
    assert_eq!(rec.validate(), Ok(()));
}

#[test]
fn a_delta_without_a_final_update_id_is_refused() {
    // Without `u` the consumer cannot chain, and an unchainable delta is worse
    // than no delta.
    let inst = spot_btcusdt();
    let frame = br#"{"e":"depthUpdate","E":1,"s":"BTCUSDT","U":1,"b":[],"a":[]}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        delta::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "u" })
    );
    assert_eq!(rec, before);
}

#[test]
fn an_empty_delta_is_a_valid_delta() {
    // Binance sends these: the book did not change but the chain advanced, and
    // a consumer that skipped it would see a gap that is not there.
    let inst = spot_btcusdt();
    let frame = br#"{"e":"depthUpdate","E":1,"s":"BTCUSDT","U":9,"u":9,"b":[],"a":[]}"#;
    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.level_count(), 0);
    assert_eq!(d.final_update_id, 9);
    assert_eq!(rec.validate(), Ok(()));
}
