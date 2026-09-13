//! `trade` — Bitget.

use crate::common::*;
use jeed_crypto::CryptoError;
use jeed_crypto::bitget::trade;
use jeed_wire::{Venue, WireKind, WireRecord, trade_kind};

#[test]
fn every_print_in_the_frame_is_decoded() {
    let inst = linear_btcusdt();
    let mut prints = trade::trades(&inst, TRADES).unwrap();
    let mut rec = WireRecord::zeroed();

    prints.decode_next(&inst, RECV_NS, &mut rec).unwrap().unwrap();
    assert_eq!(rec.kind(), Ok(WireKind::Trade));
    assert_eq!(rec.header.venue(), Ok(Venue::BitgetLinear));
    assert_eq!(rec.header.symbol_bytes(), b"BTCUSDT");
    assert_eq!(rec.header.venue_time(), Some(1_695_716_760_565_000_000));
    let t = rec.trade().unwrap();
    assert_eq!(t.price, 270_005);
    assert_eq!(t.qty, 1);
    assert_eq!(t.trade_kind, trade_kind::BUY);

    prints.decode_next(&inst, RECV_NS, &mut rec).unwrap().unwrap();
    let t = rec.trade().unwrap();
    assert_eq!(t.price, 270_000);
    assert_eq!(t.qty, 2);
    assert_eq!(t.trade_kind, trade_kind::SELL, "the taker sold");

    assert!(prints.decode_next(&inst, RECV_NS, &mut rec).is_none(), "and then the frame is spent");
}

#[test]
fn the_envelope_symbol_is_checked_once_for_the_whole_frame() {
    // Entries carry no symbol, so the check happens where the symbol is.
    let inst = linear_btcusdt();
    let frame = br#"{"action":"update","arg":{"instType":"USDT-FUTURES","channel":"trade","instId":"ETHUSDT"},"data":[{"ts":"1","price":"2700.5","size":"0.001","side":"buy"}]}"#;

    assert_eq!(trade::trades(&inst, frame).err(), Some(CryptoError::SymbolMismatch));
}

#[test]
fn a_frame_with_no_data_array_is_refused() {
    let inst = linear_btcusdt();
    let frame = br#"{"action":"update","arg":{"channel":"trade","instId":"BTCUSDT"},"code":"30001"}"#;

    assert_eq!(trade::trades(&inst, frame).err(), Some(CryptoError::Missing { key: "data" }));
}

#[test]
fn an_empty_batch_yields_no_prints_and_no_error() {
    let inst = linear_btcusdt();
    let frame = br#"{"action":"snapshot","arg":{"channel":"trade","instId":"BTCUSDT"},"data":[]}"#;

    let mut prints = trade::trades(&inst, frame).unwrap();
    let mut rec = WireRecord::zeroed();
    assert!(prints.decode_next(&inst, RECV_NS, &mut rec).is_none());
}

#[test]
fn a_side_this_build_does_not_know_still_publishes_the_print() {
    // The aggressor is one field of a print. Dropping a price and a size over
    // it would lose more than it protects.
    let inst = linear_btcusdt();
    let frame = br#"{"action":"update","arg":{"channel":"trade","instId":"BTCUSDT"},"data":[{"ts":"1695716760565","price":"27000.5","size":"0.001","side":"auction"}]}"#;

    let mut prints = trade::trades(&inst, frame).unwrap();
    let mut rec = WireRecord::zeroed();
    prints.decode_next(&inst, RECV_NS, &mut rec).unwrap().unwrap();

    assert_eq!(rec.trade().unwrap().trade_kind, trade_kind::UNKNOWN);
    assert_eq!(rec.trade().unwrap().price, 270_005);
}

#[test]
fn a_print_missing_its_price_leaves_the_record_alone() {
    let inst = linear_btcusdt();
    let frame = br#"{"action":"update","arg":{"channel":"trade","instId":"BTCUSDT"},"data":[{"ts":"1695716760565","size":"0.001","side":"buy"}]}"#;

    let mut prints = trade::trades(&inst, frame).unwrap();
    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(
        prints.decode_next(&inst, RECV_NS, &mut rec),
        Some(Err(CryptoError::Missing { key: "price" }))
    );
    assert_eq!(rec, before);
}
