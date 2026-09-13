//! `publicTrade.{symbol}` — Bybit.

use crate::common::*;
use jeed_crypto::CryptoError;
use jeed_crypto::bybit::trade;
use jeed_wire::{Scale, Venue, WireKind, WireRecord, trade_kind};

#[test]
fn every_print_in_a_frame_is_decoded() {
    let inst = linear_btcusdt();
    let mut prints = trade::trades(PUBLIC_TRADE).unwrap();
    let mut rec = WireRecord::zeroed();

    prints.decode_next(&inst, RECV_NS, &mut rec).unwrap().unwrap();
    assert_eq!(rec.kind(), Ok(WireKind::Trade));
    assert_eq!(rec.header.venue(), Ok(Venue::BybitLinear));
    assert_eq!(rec.header.symbol_bytes(), b"BTCUSDT");
    assert_eq!(rec.header.price_scale(), Ok(Scale::S2));
    assert_eq!(rec.header.qty_scale(), Ok(Scale::S3));
    assert_eq!(rec.header.venue_time(), Some(1_672_304_486_865_000_000));
    assert_eq!(rec.validate(), Ok(()));
    let t = rec.trade().unwrap();
    assert_eq!(t.price, 1_657_850);
    assert_eq!(t.qty, 1);
    assert_eq!(t.trade_kind, trade_kind::BUY);

    prints.decode_next(&inst, RECV_NS, &mut rec).unwrap().unwrap();
    let t = rec.trade().unwrap();
    assert_eq!(t.price, 1_657_800);
    assert_eq!(t.qty, 2);
    assert_eq!(t.trade_kind, trade_kind::SELL);

    assert!(prints.decode_next(&inst, RECV_NS, &mut rec).is_none());
}

#[test]
fn the_side_key_and_the_symbol_key_differ_only_in_case() {
    // `S` is the taker's side and `s` is the symbol. Reading them
    // case-insensitively would put "BTCUSDT" through the side mapping and
    // publish every print as a seller.
    let inst = linear_btcusdt();
    let frame = br#"{"topic":"publicTrade.BTCUSDT","type":"snapshot","ts":1672304486868,"data":[{"T":1672304486865,"s":"BTCUSDT","S":"Buy","v":"0.001","p":"16578.50"}]}"#;
    let mut prints = trade::trades(frame).unwrap();
    let mut rec = WireRecord::zeroed();

    prints.decode_next(&inst, RECV_NS, &mut rec).unwrap().unwrap();
    assert_eq!(rec.trade().unwrap().trade_kind, trade_kind::BUY);
    assert_eq!(rec.header.symbol_bytes(), b"BTCUSDT");
}

#[test]
fn a_print_for_another_symbol_is_refused() {
    let inst = linear_btcusdt();
    let frame = br#"{"topic":"publicTrade.ETHUSDT","type":"snapshot","ts":1672304486868,"data":[{"T":1672304486865,"s":"ETHUSDT","S":"Buy","v":"0.001","p":"1200.00"}]}"#;
    let mut prints = trade::trades(frame).unwrap();
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        prints.decode_next(&inst, RECV_NS, &mut rec),
        Some(Err(CryptoError::SymbolMismatch))
    );
    assert_eq!(rec, before);
}

#[test]
fn a_print_with_no_quantity_is_named() {
    let inst = linear_btcusdt();
    let frame = br#"{"topic":"publicTrade.BTCUSDT","type":"snapshot","ts":1672304486868,"data":[{"T":1672304486865,"s":"BTCUSDT","S":"Buy","p":"16578.50"}]}"#;
    let mut prints = trade::trades(frame).unwrap();
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        prints.decode_next(&inst, RECV_NS, &mut rec),
        Some(Err(CryptoError::Missing { key: "v" }))
    );
    assert_eq!(rec, before);
}

#[test]
fn a_frame_with_no_data_array_is_named() {
    let frame = br#"{"success":true,"op":"subscribe","conn_id":"abc"}"#;
    assert_eq!(trade::trades(frame).err(), Some(CryptoError::Missing { key: "data" }));
}
