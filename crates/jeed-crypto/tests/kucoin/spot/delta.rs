//! `/market/level2` — KuCoin spot.

use crate::common::*;
use jeed_crypto::CryptoError;
use jeed_crypto::kucoin::spot::delta;
use jeed_wire::{Scale, Venue, WIRE_MAX_DELTA_LEVELS, WireKind, WireRecord};

#[test]
fn a_diff_carries_both_sides_and_its_sequence_range() {
    let inst = spot_btc_usdt();
    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, SPOT_LEVEL2, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::SnapshotDelta));
    assert_eq!(rec.header.venue(), Ok(Venue::KucoinSpot));
    assert_eq!(rec.header.symbol_bytes(), b"BTC-USDT");
    assert_eq!(rec.header.price_scale(), Ok(Scale::S1));
    assert_eq!(rec.header.qty_scale(), Ok(Scale::S8));
    assert_eq!(rec.validate(), Ok(()));

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.bid_count, 1);
    assert_eq!(d.ask_count, 2);
    assert_eq!(d.bids()[0].price, 35_355);
    assert_eq!(d.bids()[0].qty, 3_000_000);
    assert_eq!(d.asks()[0].price, 35_361);
    assert_eq!(d.asks()[1].qty, 0, "a zero size deletes the price");
    assert_eq!(d.first_update_id, 1_545_896_669_105);
    assert_eq!(d.final_update_id, 1_545_896_669_107);
}

#[test]
fn asks_arriving_first_still_land_behind_the_bids() {
    // The `changes` object sends `asks` first, and the wire lays bids out
    // first. Nothing may assume a JSON object's key order.
    let inst = spot_btc_usdt();
    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, SPOT_LEVEL2, RECV_NS, &mut rec).unwrap();

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.levels[0].price, 35_355, "the bid is first");
    assert_eq!(d.levels[1].price, 35_361);
}

#[test]
fn the_per_level_sequence_is_stepped_over() {
    // A level is `[price, size, sequence]`. The third element must not be
    // mistaken for the next level's price.
    let inst = spot_btc_usdt();
    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, SPOT_LEVEL2, RECV_NS, &mut rec).unwrap();

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.level_count(), 3, "three levels, not six");
    assert_eq!(d.asks()[1].price, 35_365);
}

#[test]
fn this_message_carries_no_clock_so_none_is_claimed() {
    // KuCoin sends no time on this channel. `recv_ns` says when the handler
    // had it; copying that into the venue's slot would be an invention.
    let inst = spot_btc_usdt();
    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, SPOT_LEVEL2, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.venue_time(), None);
    assert_eq!(rec.header.recv_ns, RECV_NS);
}

#[test]
fn a_frame_for_another_instrument_is_refused() {
    let inst = spot_btc_usdt();
    let frame = br#"{"type":"message","topic":"/market/level2:ETH-USDT","subject":"trade.l2update","data":{"sequenceStart":1,"sequenceEnd":2,"symbol":"ETH-USDT","changes":{"asks":[],"bids":[["2000.0","1.00000000","1"]]}}}"#;

    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(delta::decode(&inst, frame, RECV_NS, &mut rec), Err(CryptoError::SymbolMismatch));
    assert_eq!(rec, before);
}

#[test]
fn a_diff_with_no_changes_object_is_refused() {
    let inst = spot_btc_usdt();
    let frame = br#"{"type":"message","topic":"/market/level2:BTC-USDT","subject":"trade.l2update","data":{"sequenceStart":1,"sequenceEnd":2,"symbol":"BTC-USDT"}}"#;

    let mut rec = WireRecord::zeroed();
    assert_eq!(
        delta::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "changes.bids" })
    );
}

#[test]
fn a_diff_missing_the_end_of_its_range_is_refused() {
    let inst = spot_btc_usdt();
    let frame = br#"{"type":"message","topic":"/market/level2:BTC-USDT","subject":"trade.l2update","data":{"sequenceStart":1,"symbol":"BTC-USDT","changes":{"asks":[],"bids":[["3535.5","0.03000000","1"]]}}}"#;

    let mut rec = WireRecord::zeroed();
    assert_eq!(
        delta::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "sequenceEnd" })
    );
}

#[test]
fn a_control_frame_is_not_a_book() {
    // `welcome`, `ack` and `pong` ride the same socket.
    let inst = spot_btc_usdt();
    let frame = br#"{"id":"1545910590801","type":"ack"}"#;

    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(
        delta::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "data" })
    );
    assert_eq!(rec, before);
}

#[test]
fn a_diff_wider_than_the_record_is_dropped_whole() {
    let inst = spot_btc_usdt();
    let bids: Vec<String> =
        (0..40).map(|i| format!(r#"["{}.5","1.00000000","{}"]"#, 3535 - i, i)).collect();
    let frame = format!(
        r#"{{"type":"message","topic":"/market/level2:BTC-USDT","subject":"trade.l2update","data":{{"sequenceStart":1,"sequenceEnd":2,"symbol":"BTC-USDT","changes":{{"asks":[],"bids":[{}]}}}}}}"#,
        bids.join(",")
    );

    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(
        delta::decode(&inst, frame.as_bytes(), RECV_NS, &mut rec),
        Err(CryptoError::DeltaOverflow { max: WIRE_MAX_DELTA_LEVELS })
    );
    assert_eq!(rec, before);
}
