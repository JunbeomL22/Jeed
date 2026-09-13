//! `@trade` — Binance spot.

use crate::common::*;
use jeed_crypto::CryptoError;
use jeed_crypto::binance::spot::trade;
use jeed_wire::{Venue, WireKind, WireRecord, header_flags, trade_kind};

#[test]
fn decodes_the_documented_frame() {
    let inst = spot_btcusdt();
    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, SPOT_TRADE, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Trade));
    assert_eq!(rec.header.venue(), Ok(Venue::BinanceSpot));
    assert_eq!(rec.header.symbol_bytes(), b"BTCUSDT");
    assert_eq!(rec.validate(), Ok(()));

    let t = rec.trade().unwrap();
    assert_eq!(t.price, 471_206, "4712.06 at two decimals");
    assert_eq!(t.qty, 384_410, "3.8441 at five decimals");
    assert_eq!(t.trade_kind, trade_kind::SELL);
}

#[test]
fn m_true_means_the_seller_aggressed() {
    // Binance reports whether the *buyer* was the maker, so the flag has to be
    // read backwards to name the aggressor. Getting this inverted would flip
    // the sign of every order-flow signal downstream.
    let inst = spot_btcusdt();
    let mut rec = WireRecord::zeroed();

    let maker_buy = br#"{"e":"trade","s":"BTCUSDT","p":"4712.06","q":"1.00000","T":1,"m":true}"#;
    trade::decode(&inst, maker_buy, RECV_NS, &mut rec).unwrap();
    assert_eq!(rec.trade().unwrap().trade_kind, trade_kind::SELL);

    let taker_buy = br#"{"e":"trade","s":"BTCUSDT","p":"4712.06","q":"1.00000","T":1,"m":false}"#;
    trade::decode(&inst, taker_buy, RECV_NS, &mut rec).unwrap();
    assert_eq!(rec.trade().unwrap().trade_kind, trade_kind::BUY);
}

#[test]
fn venue_time_comes_from_t_not_e() {
    // `T` is when the trade happened; `E` is when Binance published the event.
    // The gap between them is Binance's latency, not the market's.
    let inst = spot_btcusdt();
    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, SPOT_TRADE, RECV_NS, &mut rec).unwrap();

    assert!(rec.header.has(header_flags::VENUE_TIME_VALID));
    assert_eq!(rec.header.venue_time(), Some(1_755_088_771_744_000_000));
}

#[test]
fn a_print_with_no_time_is_still_a_print() {
    // `T` is not required: the flag says whether it was there, and refusing
    // the whole trade would lose the price for the sake of the clock.
    let inst = spot_btcusdt();
    let frame = br#"{"e":"trade","s":"BTCUSDT","p":"4712.06","q":"1.00000","m":true}"#;
    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    assert!(!rec.header.has(header_flags::VENUE_TIME_VALID));
    assert_eq!(rec.trade().unwrap().price, 471_206);
}

#[test]
fn a_trade_carries_no_book() {
    let inst = spot_btcusdt();
    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, SPOT_TRADE, RECV_NS, &mut rec).unwrap();
    assert_eq!(rec.header.depth, 0);
}

#[test]
fn cumulative_quantity_is_absent_because_binance_does_not_send_one() {
    // KRX 누적체결수량 has no counterpart here, and an unflagged zero would
    // read as "no volume today".
    let inst = spot_btcusdt();
    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, SPOT_TRADE, RECV_NS, &mut rec).unwrap();
    assert_eq!(rec.trade().unwrap().cumulative_qty(), None);
}

#[test]
fn a_missing_aggressor_flag_is_named() {
    let inst = spot_btcusdt();
    let frame = br#"{"e":"trade","s":"BTCUSDT","p":"4712.06","q":"1.00000","T":1}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        trade::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "m" })
    );
    assert_eq!(rec, before);
}
