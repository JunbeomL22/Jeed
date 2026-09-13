//! `trades` — OKX.

use crate::common::*;
use jeed_crypto::CryptoError;
use jeed_crypto::okx::trade;
use jeed_wire::{Scale, Venue, WireKind, WireRecord, trade_kind};

#[test]
fn every_print_in_a_frame_is_decoded() {
    // `fractal-engine` kept the last print of a batch and dropped the rest.
    // Every one of them moved the tape.
    let inst = btc_usdt();
    let mut prints = trade::trades(TRADES).unwrap();
    let mut rec = WireRecord::zeroed();

    prints.decode_next(&inst, RECV_NS, &mut rec).unwrap().unwrap();
    assert_eq!(rec.kind(), Ok(WireKind::Trade));
    assert_eq!(rec.header.venue(), Ok(Venue::Okx));
    assert_eq!(rec.header.symbol_bytes(), b"BTC-USDT");
    assert_eq!(rec.header.price_scale(), Ok(Scale::S1));
    assert_eq!(rec.header.qty_scale(), Ok(Scale::S8));
    assert_eq!(rec.header.venue_time(), Some(1_706_000_000_000_000_000));
    assert_eq!(rec.validate(), Ok(()));
    let t = rec.trade().unwrap();
    assert_eq!(t.price, 640_005);
    assert_eq!(t.qty, 12_000_000);
    assert_eq!(t.trade_kind, trade_kind::BUY);

    prints.decode_next(&inst, RECV_NS, &mut rec).unwrap().unwrap();
    let t = rec.trade().unwrap();
    assert_eq!(t.price, 640_004);
    assert_eq!(t.qty, 3_000_000);
    assert_eq!(t.trade_kind, trade_kind::SELL);
    assert_eq!(rec.header.venue_time(), Some(1_706_000_000_001_000_000));

    assert!(prints.decode_next(&inst, RECV_NS, &mut rec).is_none());
}

#[test]
fn an_empty_batch_yields_nothing_and_is_not_an_error() {
    let inst = btc_usdt();
    let frame = br#"{"arg":{"channel":"trades","instId":"BTC-USDT"},"data":[]}"#;
    let mut prints = trade::trades(frame).unwrap();
    let mut rec = WireRecord::zeroed();

    assert!(prints.decode_next(&inst, RECV_NS, &mut rec).is_none());
}

#[test]
fn a_frame_with_no_data_array_is_named() {
    let frame = br#"{"event":"subscribe","arg":{"channel":"trades","instId":"BTC-USDT"}}"#;
    assert_eq!(
        trade::trades(frame).err(),
        Some(CryptoError::Missing { key: "data" })
    );
}

#[test]
fn a_print_for_another_instrument_is_refused_and_the_batch_goes_on() {
    // Each entry names its own `instId`, so a mis-wired stream is caught per
    // print rather than per frame — and one bad entry does not make the
    // entries after it wrong.
    let inst = btc_usdt();
    let frame = br#"{"arg":{"channel":"trades","instId":"BTC-USDT"},"data":[{"instId":"ETH-USDT","px":"3000.0","sz":"1.00000000","side":"buy","ts":"1706000000000"},{"instId":"BTC-USDT","px":"64000.5","sz":"0.01000000","side":"sell","ts":"1706000000001"}]}"#;
    let mut prints = trade::trades(frame).unwrap();
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        prints.decode_next(&inst, RECV_NS, &mut rec),
        Some(Err(CryptoError::SymbolMismatch))
    );
    assert_eq!(rec, before, "the refused print left the slot alone");

    prints.decode_next(&inst, RECV_NS, &mut rec).unwrap().unwrap();
    assert_eq!(rec.trade().unwrap().price, 640_005);
}

#[test]
fn a_print_with_no_price_is_named() {
    let inst = btc_usdt();
    let frame = br#"{"arg":{"channel":"trades","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","sz":"0.01000000","side":"buy","ts":"1706000000000"}]}"#;
    let mut prints = trade::trades(frame).unwrap();
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        prints.decode_next(&inst, RECV_NS, &mut rec),
        Some(Err(CryptoError::Missing { key: "px" }))
    );
    assert_eq!(rec, before);
}

#[test]
fn an_unreadable_side_does_not_cost_the_print() {
    let inst = btc_usdt();
    let frame = br#"{"arg":{"channel":"trades","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","px":"64000.5","sz":"0.01000000","ts":"1706000000000"}]}"#;
    let mut prints = trade::trades(frame).unwrap();
    let mut rec = WireRecord::zeroed();

    prints.decode_next(&inst, RECV_NS, &mut rec).unwrap().unwrap();
    assert_eq!(rec.trade().unwrap().trade_kind, trade_kind::UNKNOWN);
    assert_eq!(rec.trade().unwrap().price, 640_005);
}
