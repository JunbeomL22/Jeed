//! `books` / `books5` — OKX.

use crate::common::*;
use jeed_crypto::CryptoError;
use jeed_crypto::okx::book;
use jeed_wire::{
    Scale, Venue, WIRE_MAX_DELTA_LEVELS, WIRE_MAX_DEPTH, WireKind, WireRecord, delta_flags,
    quote_ext,
};

#[test]
fn a_snapshot_action_becomes_a_whole_book() {
    let inst = btc_usdt();
    let mut rec = WireRecord::zeroed();
    book::decode(&inst, BOOKS_SNAPSHOT, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.venue(), Ok(Venue::Okx));
    assert_eq!(rec.header.symbol_bytes(), b"BTC-USDT");
    assert_eq!(rec.header.price_scale(), Ok(Scale::S1));
    assert_eq!(rec.header.qty_scale(), Ok(Scale::S8));
    assert_eq!(rec.header.depth, 2);
    assert_eq!(rec.header.venue_time(), Some(1_706_000_000_000_000_000));
    assert_eq!(rec.validate(), Ok(()));

    let q = rec.quote().unwrap();
    assert_eq!(q.ask[0].price, 640_005);
    assert_eq!(q.ask[0].qty, 1_000_000);
    assert_eq!(q.bid[0].price, 639_995);
    assert_eq!(q.bid[1].qty, 10_000_000);
    assert_eq!(q.quote_ext_kind, quote_ext::SEQUENCE);
    assert_eq!(q.quote_ext, 1_234_567_890);
}

#[test]
fn the_deprecated_third_element_and_the_order_count_are_stepped_over() {
    // A level is `[price, size, "0", order count]`. Only the first two are
    // read, and the reader must not mistake the trailing pair for a level.
    let inst = btc_usdt();
    let mut rec = WireRecord::zeroed();
    book::decode(&inst, BOOKS_SNAPSHOT, RECV_NS, &mut rec).unwrap();

    let q = rec.quote().unwrap();
    assert_eq!(q.ask[1].price, 640_010, "the second ask, not the count of the first");
    assert_eq!(q.ask[1].order_count, 0, "counts are not carried");
    assert_eq!(q.ask[2], Default::default());
}

#[test]
fn an_update_action_becomes_a_delta() {
    let inst = btc_usdt();
    let mut rec = WireRecord::zeroed();
    book::decode(&inst, BOOKS_UPDATE, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::SnapshotDelta));
    assert_eq!(rec.validate(), Ok(()));

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.bid_count, 2);
    assert_eq!(d.ask_count, 1);
    assert_eq!(d.level_count(), 3);
    assert_eq!(d.bids()[0].price, 639_995);
    assert_eq!(d.bids()[1].price, 639_980);
    assert_eq!(d.asks()[0].price, 640_020);
    assert_eq!(d.asks()[0].qty, 0, "a zero size deletes the price");
}

#[test]
fn asks_arriving_first_still_land_behind_the_bids() {
    // OKX sends `asks` before `bids` and the wire lays bids out first, so the
    // shared array is ordered after both sides are in. Nothing may assume a
    // JSON object's key order.
    let inst = btc_usdt();
    let mut rec = WireRecord::zeroed();
    book::decode(&inst, BOOKS_UPDATE, RECV_NS, &mut rec).unwrap();

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.levels[0].price, 639_995, "first slot is a bid");
    assert_eq!(d.levels[1].price, 639_980);
    assert_eq!(d.levels[2].price, 640_020, "the ask follows them");
}

#[test]
fn the_sequence_chain_is_carried_the_way_okx_spells_it() {
    // One sequence number per message, so both ends of the range are `seqId`;
    // `prevSeqId` names the message this one must follow and rides in the
    // slot Binance USD-M's `pu` uses, because it answers the same question.
    let inst = btc_usdt();
    let mut rec = WireRecord::zeroed();
    book::decode(&inst, BOOKS_UPDATE, RECV_NS, &mut rec).unwrap();

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.first_update_id, 1_234_567_891);
    assert_eq!(d.final_update_id, 1_234_567_891);
    assert_eq!(d.prev_final_update_id(), Some(1_234_567_890));
    assert_ne!(d.delta_flags & delta_flags::PREV_FINAL_VALID, 0);
}

#[test]
fn books5_has_no_action_and_is_a_whole_book() {
    let inst = btc_usdt();
    let mut rec = WireRecord::zeroed();
    book::decode(&inst, BOOKS5, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.depth, 1);
    assert_eq!(rec.quote().unwrap().quote_ext, 1_234_567_890);
}

#[test]
fn an_action_this_build_does_not_know_publishes_nothing() {
    // There is no safe default: a diff read as a snapshot throws the book
    // away, a snapshot read as a diff leaves every other price untouched
    // forever.
    let inst = btc_usdt();
    let frame = br#"{"arg":{"channel":"books","instId":"BTC-USDT"},"action":"replace","data":[{"asks":[],"bids":[],"ts":"1706000000000","seqId":1,"prevSeqId":0}]}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        book::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Unexpected { key: "action" })
    );
    assert_eq!(rec, before);
}

#[test]
fn an_empty_update_is_a_keep_alive() {
    // OKX repeats the last `seqId` with both sides empty when nothing has
    // changed, to show the connection is alive. That is a delta with no
    // levels and a chain that does not advance — exactly what happened.
    let inst = btc_usdt();
    let frame = br#"{"arg":{"channel":"books","instId":"BTC-USDT"},"action":"update","data":[{"asks":[],"bids":[],"ts":"1706000000200","checksum":0,"seqId":1234567891,"prevSeqId":1234567891}]}"#;
    let mut rec = WireRecord::zeroed();
    book::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.level_count(), 0);
    assert_eq!(d.final_update_id, d.prev_final_update_id().unwrap());
}

#[test]
fn a_diff_with_no_predecessor_named_is_refused() {
    // Without `prevSeqId` the consumer cannot tell a delta it may apply from
    // one that would corrupt its book.
    let inst = btc_usdt();
    let frame = br#"{"arg":{"channel":"books","instId":"BTC-USDT"},"action":"update","data":[{"asks":[],"bids":[["63999.5","0.05000000","0","2"]],"ts":"1706000000100","seqId":1234567891}]}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        book::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "prevSeqId" })
    );
    assert_eq!(rec, before);
}

#[test]
fn a_frame_for_another_instrument_is_refused_from_the_envelope() {
    // A `books` data object carries no `instId`; `arg` is the only place the
    // subscription names itself.
    let inst = btc_usdt();
    let frame = br#"{"arg":{"channel":"books","instId":"ETH-USDT"},"action":"snapshot","data":[{"asks":[["3000.0","1.00000000","0","1"]],"bids":[["2999.0","1.00000000","0","1"]],"ts":"1706000000000","seqId":1,"prevSeqId":0}]}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(book::decode(&inst, frame, RECV_NS, &mut rec), Err(CryptoError::SymbolMismatch));
    assert_eq!(rec, before);
}

#[test]
fn a_snapshot_deeper_than_the_wire_is_trimmed() {
    // Shallower is not wrong. `books` sends up to four hundred levels.
    let inst = btc_usdt();
    let mut frame =
        br#"{"arg":{"channel":"books","instId":"BTC-USDT"},"action":"snapshot","data":[{"asks":["#
            .to_vec();
    for i in 0..40u64 {
        if i > 0 {
            frame.push(b',');
        }
        frame.extend_from_slice(
            format!(r#"["{}.0","1.00000000","0","1"]"#, 64_000 + i).as_bytes(),
        );
    }
    frame.extend_from_slice(
        br#"],"bids":[["63999.5","1.00000000","0","1"]],"ts":"1706000000000","seqId":1,"prevSeqId":0}]}"#,
    );

    let mut rec = WireRecord::zeroed();
    book::decode(&inst, &frame, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.depth as usize, WIRE_MAX_DEPTH);
    assert_eq!(rec.quote().unwrap().ask[WIRE_MAX_DEPTH - 1].price, 640_090);
}

#[test]
fn a_diff_deeper_than_the_wire_is_dropped_whole() {
    // The opposite rule to a snapshot, on purpose: a truncated diff is
    // accepted as complete and the lost changes are never re-sent, while a
    // dropped one leaves a hole the consumer already knows how to repair.
    let inst = btc_usdt();
    let mut frame =
        br#"{"arg":{"channel":"books","instId":"BTC-USDT"},"action":"update","data":[{"asks":[],"bids":["#
            .to_vec();
    for i in 0..(WIRE_MAX_DELTA_LEVELS as u64 + 1) {
        if i > 0 {
            frame.push(b',');
        }
        frame.extend_from_slice(
            format!(r#"["{}.0","1.00000000","0","1"]"#, 63_000 + i).as_bytes(),
        );
    }
    frame.extend_from_slice(br#"],"ts":"1706000000100","seqId":2,"prevSeqId":1}]}"#);

    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        book::decode(&inst, &frame, RECV_NS, &mut rec),
        Err(CryptoError::DeltaOverflow { max: WIRE_MAX_DELTA_LEVELS })
    );
    assert_eq!(rec, before, "nothing is published");
}

#[test]
fn a_frame_with_no_data_array_is_named() {
    let inst = btc_usdt();
    let frame = br#"{"arg":{"channel":"books","instId":"BTC-USDT"},"action":"snapshot"}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        book::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "data" })
    );
    assert_eq!(rec, before);
}
