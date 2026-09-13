//! `books` family — Bitget.

use crate::common::*;
use jeed_crypto::CryptoError;
use jeed_crypto::bitget::book;
use jeed_wire::{
    Scale, Venue, WIRE_MAX_DELTA_LEVELS, WireKind, WireRecord, delta_flags, quote_ext,
};

#[test]
fn a_snapshot_action_becomes_a_whole_book() {
    let inst = linear_btcusdt();
    let mut rec = WireRecord::zeroed();
    book::decode(&inst, BOOKS_SNAPSHOT, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.venue(), Ok(Venue::BitgetLinear));
    assert_eq!(rec.header.symbol_bytes(), b"BTCUSDT");
    assert_eq!(rec.header.price_scale(), Ok(Scale::S1));
    assert_eq!(rec.header.qty_scale(), Ok(Scale::S3));
    assert_eq!(rec.header.depth, 2);
    assert_eq!(rec.header.venue_time(), Some(1_695_716_059_616_000_000));
    assert_eq!(rec.validate(), Ok(()));

    let q = rec.quote().unwrap();
    assert_eq!(q.ask[0].price, 270_005);
    assert_eq!(q.ask[0].qty, 8_760);
    assert_eq!(q.bid[0].price, 270_000);
    assert_eq!(q.bid[1].qty, 1_000);
    assert_eq!(q.quote_ext_kind, quote_ext::SEQUENCE);
    assert_eq!(q.quote_ext, 456, "the quoted `seq`, read as a number");
}

#[test]
fn the_same_symbol_on_the_other_market_is_a_different_record() {
    // `BTCUSDT` is the spot pair and the perpetual. The frames are identical
    // and only the instrument's venue byte keeps the two books apart.
    let mut linear = WireRecord::zeroed();
    let mut spot = WireRecord::zeroed();
    book::decode(&linear_btcusdt(), BOOKS_SNAPSHOT, RECV_NS, &mut linear).unwrap();
    book::decode(&spot_btcusdt(), BOOKS_SNAPSHOT, RECV_NS, &mut spot).unwrap();

    assert_ne!(linear, spot);
    assert_eq!(linear.header.venue(), Ok(Venue::BitgetLinear));
    assert_eq!(spot.header.venue(), Ok(Venue::BitgetSpot));

    linear.header.venue = spot.header.venue;
    assert_eq!(linear, spot, "the venue byte is the whole difference");
}

#[test]
fn an_update_action_becomes_a_delta() {
    let inst = linear_btcusdt();
    let mut rec = WireRecord::zeroed();
    book::decode(&inst, BOOKS_UPDATE, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::SnapshotDelta));
    assert_eq!(rec.validate(), Ok(()));

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.bid_count, 2);
    assert_eq!(d.ask_count, 1);
    assert_eq!(d.bids()[0].price, 270_000);
    assert_eq!(d.bids()[0].qty, 3_100);
    assert_eq!(d.asks()[0].price, 270_030);
    assert_eq!(d.asks()[0].qty, 0, "a zero size deletes the price");
}

#[test]
fn asks_arriving_first_still_land_behind_the_bids() {
    let inst = linear_btcusdt();
    let mut rec = WireRecord::zeroed();
    book::decode(&inst, BOOKS_UPDATE, RECV_NS, &mut rec).unwrap();

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.levels[0].price, 270_000, "first slot is a bid");
    assert_eq!(d.levels[2].price, 270_030, "the ask follows them");
}

#[test]
fn the_sequence_chain_is_carried_the_way_bitget_spells_it() {
    // `pseq → seq` is OKX's `prevSeqId → seqId`: one message covers one
    // update, so both ends of the range are `seq`, and `pseq` names the
    // message this one must follow.
    let inst = linear_btcusdt();
    let mut rec = WireRecord::zeroed();
    book::decode(&inst, BOOKS_UPDATE, RECV_NS, &mut rec).unwrap();

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.first_update_id, 457);
    assert_eq!(d.final_update_id, 457);
    assert_eq!(d.prev_final_update_id(), Some(456));
    assert!(d.delta_flags & delta_flags::PREV_FINAL_VALID != 0);
}

#[test]
fn a_shallow_channel_is_a_whole_book_too() {
    // `books5` types every push `snapshot`, and that is what it is.
    let inst = spot_btcusdt();
    let mut rec = WireRecord::zeroed();
    book::decode(&inst, BOOKS5, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.depth, 1);
    assert_eq!(rec.quote().unwrap().quote_ext, 99);
}

#[test]
fn an_absent_action_is_refused() {
    // OKX's `books5` has no `action`; every Bitget book message has one, so a
    // frame without it is a protocol change rather than a shallow push.
    let inst = linear_btcusdt();
    let frame = br#"{"arg":{"channel":"books","instId":"BTCUSDT"},"data":[{"asks":[],"bids":[],"seq":"1","ts":"1"}]}"#;

    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(
        book::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "action" })
    );
    assert_eq!(rec, before);
}

#[test]
fn an_action_this_build_does_not_know_publishes_nothing() {
    let inst = linear_btcusdt();
    let frame = br#"{"action":"patch","arg":{"channel":"books","instId":"BTCUSDT"},"data":[{"asks":[],"bids":[],"seq":"1","pseq":"0","ts":"1"}]}"#;

    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(
        book::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Unexpected { key: "action" })
    );
    assert_eq!(rec, before, "there is no safe guess between whole and diff");
}

#[test]
fn a_frame_for_another_instrument_is_refused() {
    // The `data` objects carry no symbol, so `arg.instId` is the only place a
    // mis-wired subscription can be caught.
    let inst = linear_btcusdt();
    let frame = br#"{"action":"snapshot","arg":{"instType":"USDT-FUTURES","channel":"books","instId":"ETHUSDT"},"data":[{"asks":[["2700.5","1.000"]],"bids":[["2700.0","1.000"]],"seq":"1","ts":"1"}]}"#;

    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(book::decode(&inst, frame, RECV_NS, &mut rec), Err(CryptoError::SymbolMismatch));
    assert_eq!(rec, before);
}

#[test]
fn a_diff_without_its_predecessor_is_refused() {
    // Without `pseq` the consumer cannot tell a delta it may apply from one
    // that would corrupt its book.
    let inst = linear_btcusdt();
    let frame = br#"{"action":"update","arg":{"channel":"books","instId":"BTCUSDT"},"data":[{"asks":[],"bids":[["27000.0","1.000"]],"seq":"457","ts":"1"}]}"#;

    let mut rec = WireRecord::zeroed();
    assert_eq!(
        book::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "pseq" })
    );
}

#[test]
fn a_whole_book_does_not_need_a_predecessor() {
    // It replaces everything, so there is nothing for it to follow.
    let inst = linear_btcusdt();
    let frame = br#"{"action":"snapshot","arg":{"channel":"books","instId":"BTCUSDT"},"data":[{"asks":[["27001.0","1.000"]],"bids":[["27000.0","2.000"]],"seq":"456","ts":"1695716059616"}]}"#;

    let mut rec = WireRecord::zeroed();
    book::decode(&inst, frame, RECV_NS, &mut rec).unwrap();
    assert_eq!(rec.kind(), Ok(WireKind::Quote));
}

#[test]
fn a_diff_wider_than_the_record_is_dropped_whole() {
    let inst = linear_btcusdt();
    let bids: Vec<String> = (0..40).map(|i| format!(r#"["{}.0","1.000"]"#, 27000 - i)).collect();
    let frame = format!(
        r#"{{"action":"update","arg":{{"channel":"books","instId":"BTCUSDT"}},"data":[{{"asks":[],"bids":[{}],"seq":"2","pseq":"1","ts":"1"}}]}}"#,
        bids.join(",")
    );

    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(
        book::decode(&inst, frame.as_bytes(), RECV_NS, &mut rec),
        Err(CryptoError::DeltaOverflow { max: WIRE_MAX_DELTA_LEVELS })
    );
    assert_eq!(rec, before, "a truncated delta is wrong forever; a dropped one resyncs");
}

#[test]
fn an_empty_diff_is_a_keep_alive_and_not_a_fault() {
    let inst = linear_btcusdt();
    let frame = br#"{"action":"update","arg":{"channel":"books","instId":"BTCUSDT"},"data":[{"asks":[],"bids":[],"seq":"457","pseq":"457","ts":"1695716059716"}]}"#;

    let mut rec = WireRecord::zeroed();
    book::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.level_count(), 0);
    assert_eq!(d.first_update_id, 457, "the chain does not advance, which is what happened");
    assert_eq!(d.prev_final_update_id(), Some(457));
}
