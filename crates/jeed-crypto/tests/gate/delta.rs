//! `spot.order_book_update` — Gate.

use crate::common::*;
use jeed_crypto::CryptoError;
use jeed_crypto::gate::delta;
use jeed_wire::{Scale, Venue, WIRE_MAX_DELTA_LEVELS, WireKind, WireRecord, delta_flags};

#[test]
fn a_diff_carries_both_sides_and_its_update_range() {
    let inst = btc_usdt();
    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, ORDER_BOOK_UPDATE, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::SnapshotDelta));
    assert_eq!(rec.header.venue(), Ok(Venue::GateSpot));
    assert_eq!(rec.header.symbol_bytes(), b"BTC_USDT");
    assert_eq!(rec.header.price_scale(), Ok(Scale::S2));
    assert_eq!(rec.header.qty_scale(), Ok(Scale::S4));
    assert_eq!(rec.validate(), Ok(()));

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.bid_count, 2);
    assert_eq!(d.ask_count, 1);
    assert_eq!(d.bids()[0].price, 1_913_774);
    assert_eq!(d.bids()[0].qty, 1);
    assert_eq!(d.bids()[1].qty, 0, "a zero amount deletes the price");
    assert_eq!(d.asks()[0].price, 1_913_775);
    assert_eq!(d.first_update_id, 48_776_301);
    assert_eq!(d.final_update_id, 48_776_306);
}

#[test]
fn the_book_clock_is_carried_and_not_the_push_clock() {
    // `t` is when the book changed; `time_ms` is when Gate got round to
    // sending it. `venue_ns` is meant to compare with another venue's record
    // of the same market move.
    let inst = btc_usdt();
    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, ORDER_BOOK_UPDATE, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.venue_time(), Some(1_606_294_781_123_000_000));
    assert_eq!(rec.header.recv_ns, RECV_NS);
}

#[test]
fn there_is_no_named_predecessor_so_the_slot_stays_empty() {
    // Gate chains on `U`/`u` as Binance spot does. `U - 1` would be an
    // invention, and the flag exists so that absence can be said out loud.
    let inst = btc_usdt();
    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, ORDER_BOOK_UPDATE, RECV_NS, &mut rec).unwrap();

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.prev_final_update_id(), None);
    assert!(d.delta_flags & delta_flags::PREV_FINAL_VALID == 0);
    assert_eq!(d.first_update_id, 48_776_301, "the range itself is there");
}

#[test]
fn a_subscribe_answer_is_not_a_book() {
    // Control frames ride the same socket with the same envelope. Reading the
    // fields a book needs reaches the same verdict an `event` check would.
    let inst = btc_usdt();
    let mut rec = dirty_record();
    let before = rec;

    assert!(matches!(
        delta::decode(&inst, SUBSCRIBE_ACK, RECV_NS, &mut rec),
        Err(CryptoError::Missing { .. })
    ));
    assert_eq!(rec, before);
}

#[test]
fn a_frame_with_no_result_object_is_refused() {
    let inst = btc_usdt();
    let frame = br#"{"time":1606294781,"channel":"spot.order_book_update","event":"update","error":{"code":2,"message":"unknown"}}"#;

    let mut rec = WireRecord::zeroed();
    assert_eq!(
        delta::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "result" })
    );
}

#[test]
fn a_frame_for_another_instrument_is_refused() {
    let inst = btc_usdt();
    let frame = br#"{"channel":"spot.order_book_update","event":"update","result":{"t":1,"s":"ETH_USDT","U":1,"u":2,"b":[["1913.77","0.1000"]],"a":[]}}"#;

    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(delta::decode(&inst, frame, RECV_NS, &mut rec), Err(CryptoError::SymbolMismatch));
    assert_eq!(rec, before);
}

#[test]
fn a_diff_missing_its_first_update_id_is_refused() {
    // Without `U` the consumer cannot tell whether the diff it is holding
    // follows the snapshot it has.
    let inst = btc_usdt();
    let frame = br#"{"channel":"spot.order_book_update","event":"update","result":{"t":1,"s":"BTC_USDT","u":2,"b":[["19137.74","0.0001"]],"a":[]}}"#;

    let mut rec = WireRecord::zeroed();
    assert_eq!(
        delta::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "U" })
    );
}

#[test]
fn a_diff_wider_than_the_record_is_dropped_whole() {
    let inst = btc_usdt();
    let bids: Vec<String> = (0..40).map(|i| format!(r#"["{}.00","1.0000"]"#, 19137 - i)).collect();
    let frame = format!(
        r#"{{"channel":"spot.order_book_update","event":"update","result":{{"t":1,"s":"BTC_USDT","U":1,"u":2,"b":[{}],"a":[]}}}}"#,
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
