//! `orderbook` — Upbit.

use crate::common::*;
use jeed_convert::ParseErr;
use jeed_crypto::upbit::snapshot;
use jeed_crypto::{CryptoError, Instrument};
use jeed_wire::{Scale, Venue, WIRE_MAX_DEPTH, WireKind, WireRecord, header_flags, quote_ext};

#[test]
fn decodes_the_documented_frame() {
    let inst = krw_btc();
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, ORDERBOOK, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.venue(), Ok(Venue::Upbit));
    assert_eq!(rec.header.symbol_bytes(), b"KRW-BTC");
    assert_eq!(rec.header.recv_ns, RECV_NS);
    assert_eq!(rec.header.price_scale(), Ok(Scale::S0));
    assert_eq!(rec.header.qty_scale(), Ok(Scale::S8));
    assert_eq!(rec.header.depth, 2);
    assert_eq!(rec.header.venue_time(), Some(1_704_067_200_000_000_000));
    assert_eq!(rec.validate(), Ok(()));

    let q = rec.quote().unwrap();
    // KRW prices are whole won at S0; 0.17 at eight places is 17_000_000.
    assert_eq!(q.bid[0].price, 152_400_000);
    assert_eq!(q.bid[0].qty, 23_000_000);
    assert_eq!(q.ask[0].price, 152_430_000);
    assert_eq!(q.ask[0].qty, 17_000_000);
    assert_eq!(q.bid[1].price, 152_390_000);
    assert_eq!(q.ask[1].price, 152_440_000);
}

#[test]
fn the_simple_spelling_decodes_to_the_same_record() {
    // A subscription asks for one spelling and gets it for the whole
    // connection, so both have to work and both have to agree.
    let inst = krw_btc();

    let mut a = WireRecord::zeroed();
    snapshot::decode(&inst, ORDERBOOK, RECV_NS, &mut a).unwrap();
    let mut b = WireRecord::zeroed();
    snapshot::decode(&inst, ORDERBOOK_SIMPLE, RECV_NS, &mut b).unwrap();

    assert_eq!(a, b);
}

#[test]
fn no_sequence_is_invented_for_a_venue_that_has_none() {
    // Upbit publishes no book sequence. `fractal-engine` echoed the
    // millisecond timestamp into the slot; a consumer would then compare two
    // books published in the same millisecond and conclude they were one book.
    let inst = krw_btc();
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, ORDERBOOK, RECV_NS, &mut rec).unwrap();

    let q = rec.quote().unwrap();
    assert_eq!(q.quote_ext_kind, quote_ext::NONE);
    assert_eq!(q.quote_ext, 0);
}

#[test]
fn the_two_sides_are_counted_apart() {
    // Units are paired, so a thin side runs out first and arrives padded with
    // zeros. Depth is levels carrying quantity, not units received.
    let inst = krw_btc();
    let frame = br#"{"type":"orderbook","code":"KRW-BTC","timestamp":1704067200000,"orderbook_units":[{"ask_price":152430000.0,"bid_price":152400000.0,"ask_size":0.17,"bid_size":0.23},{"ask_price":152440000.0,"bid_price":0.0,"ask_size":0.05,"bid_size":0.0}]}"#;
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.depth, 2, "the ask side reaches rank two");
    assert!(!rec.header.has(header_flags::BID_EMPTY));
    let q = rec.quote().unwrap();
    assert_eq!(q.bid[1].qty, 0);
}

#[test]
fn a_book_with_nothing_in_it_is_flagged_on_both_sides() {
    let inst = krw_btc();
    let frame = br#"{"type":"orderbook","code":"KRW-BTC","timestamp":1704067200000,"orderbook_units":[]}"#;
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.depth, 0);
    assert!(rec.header.has(header_flags::BID_EMPTY));
    assert!(rec.header.has(header_flags::ASK_EMPTY));
}

#[test]
fn ranks_past_the_wire_depth_are_dropped() {
    // A subscription can ask for fifteen or thirty; the wire carries ten.
    let inst = krw_btc();
    let mut frame =
        br#"{"type":"orderbook","code":"KRW-BTC","timestamp":1704067200000,"orderbook_units":["#
            .to_vec();
    for i in 0..15u64 {
        if i > 0 {
            frame.push(b',');
        }
        frame.extend_from_slice(
            format!(
                r#"{{"ask_price":{}.0,"bid_price":{}.0,"ask_size":0.1,"bid_size":0.1}}"#,
                152_430_000 + i * 1_000,
                152_400_000 - i * 1_000
            )
            .as_bytes(),
        );
    }
    frame.extend_from_slice(b"]}");

    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, &frame, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.depth as usize, WIRE_MAX_DEPTH);
    let q = rec.quote().unwrap();
    assert_eq!(q.ask[WIRE_MAX_DEPTH - 1].price, 152_430_000 + 9_000);
}

#[test]
fn a_frame_for_another_market_is_refused() {
    let inst = krw_btc();
    let frame = br#"{"type":"orderbook","code":"KRW-ETH","timestamp":1704067200000,"orderbook_units":[{"ask_price":5000000.0,"bid_price":4999000.0,"ask_size":0.1,"bid_size":0.1}]}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        snapshot::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::SymbolMismatch)
    );
    assert_eq!(rec, before, "a refused frame leaves the slot alone");
}

#[test]
fn a_frame_with_no_units_is_named() {
    let inst = krw_btc();
    let frame = br#"{"type":"orderbook","code":"KRW-BTC","timestamp":1704067200000}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        snapshot::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "orderbook_units" })
    );
    assert_eq!(rec, before);
}

#[test]
fn a_price_finer_than_the_scale_fails_rather_than_rounds() {
    // KRW-BTC is configured whole-won. A fractional price means the market's
    // tick changed, and a truncated price would be a plausible wrong number.
    let inst = krw_btc();
    let frame = br#"{"type":"orderbook","code":"KRW-BTC","timestamp":1704067200000,"orderbook_units":[{"ask_price":152430000.5,"bid_price":152400000.0,"ask_size":0.17,"bid_size":0.23}]}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        snapshot::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Field { key: "ask_price", err: ParseErr::Precision })
    );
    assert_eq!(rec, before);
}

#[test]
fn a_krw_market_with_a_fractional_tick_is_just_a_different_scale() {
    // Not every KRW market is whole-won: the cheap ones tick in fractions of
    // a won, and that is configuration, not a special case in the decoder.
    let inst = Instrument::new(Venue::Upbit, b"KRW-XRP", 1, 8).unwrap();
    let frame = br#"{"type":"orderbook","code":"KRW-XRP","timestamp":1704067200000,"orderbook_units":[{"ask_price":812.5,"bid_price":812.4,"ask_size":1000.0,"bid_size":2000.0}]}"#;
    let mut rec = WireRecord::zeroed();
    snapshot::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    let q = rec.quote().unwrap();
    assert_eq!(q.ask[0].price, 8_125);
    assert_eq!(q.bid[0].price, 8_124);
    assert_eq!(q.ask[0].qty, 100_000_000_000);
}
