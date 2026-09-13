//! `trade` — Upbit.

use crate::common::*;
use jeed_crypto::CryptoError;
use jeed_crypto::upbit::trade;
use jeed_wire::{Scale, Venue, WireKind, WireRecord, trade_kind};

#[test]
fn decodes_the_documented_frame() {
    let inst = krw_btc();
    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, TRADE, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Trade));
    assert_eq!(rec.header.venue(), Ok(Venue::Upbit));
    assert_eq!(rec.header.symbol_bytes(), b"KRW-BTC");
    assert_eq!(rec.header.price_scale(), Ok(Scale::S0));
    assert_eq!(rec.header.qty_scale(), Ok(Scale::S8));
    assert_eq!(rec.validate(), Ok(()));

    let t = rec.trade().unwrap();
    assert_eq!(t.price, 152_430_000);
    assert_eq!(t.qty, 84_280);
    assert_eq!(t.trade_kind, trade_kind::BUY, "BID is an aggressive buyer");
}

#[test]
fn the_venue_time_is_when_the_trade_happened() {
    // `trade_timestamp` is the execution; `timestamp` is when Upbit pushed
    // the message. Only the first is comparable with another venue's print.
    let inst = krw_btc();
    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, TRADE, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.venue_time(), Some(1_704_067_200_000_000_000));
    assert_ne!(rec.header.venue_ns, 1_704_067_200_123 * 1_000_000);
}

#[test]
fn the_simple_spelling_decodes_and_ask_is_a_seller() {
    let inst = krw_btc();
    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, TRADE_SIMPLE, RECV_NS, &mut rec).unwrap();

    let t = rec.trade().unwrap();
    assert_eq!(t.price, 152_430_000);
    assert_eq!(t.qty, 84_280);
    assert_eq!(t.trade_kind, trade_kind::SELL, "ASK is an aggressive seller");
    assert_eq!(rec.header.venue_time(), Some(1_704_067_200_000_000_000));
}

#[test]
fn an_unreadable_side_does_not_cost_the_print() {
    // The aggressor is one field of a print. `UNKNOWN` says the flag was there
    // and said nothing, which is more than dropping the price and size.
    let inst = krw_btc();
    let frame = br#"{"type":"trade","code":"KRW-BTC","trade_timestamp":1704067200000,"trade_price":152430000.0,"trade_volume":0.001,"ask_bid":"SOMETHING"}"#;
    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.trade().unwrap().trade_kind, trade_kind::UNKNOWN);
    assert_eq!(rec.trade().unwrap().price, 152_430_000);
}

#[test]
fn a_print_with_no_time_is_still_a_print() {
    let inst = krw_btc();
    let frame = br#"{"type":"trade","code":"KRW-BTC","trade_price":152430000.0,"trade_volume":0.001,"ask_bid":"BID"}"#;
    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.venue_time(), None);
    assert_eq!(rec.trade().unwrap().qty, 100_000);
}

#[test]
fn a_missing_field_is_named() {
    let inst = krw_btc();
    let frame = br#"{"type":"trade","code":"KRW-BTC","trade_price":152430000.0,"ask_bid":"BID"}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        trade::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "trade_volume" })
    );
    assert_eq!(rec, before);
}

#[test]
fn a_frame_for_another_market_is_refused() {
    let inst = krw_btc();
    let frame = br#"{"type":"trade","code":"KRW-ETH","trade_price":5000000.0,"trade_volume":0.001,"ask_bid":"BID"}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(trade::decode(&inst, frame, RECV_NS, &mut rec), Err(CryptoError::SymbolMismatch));
    assert_eq!(rec, before);
}
