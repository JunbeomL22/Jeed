//! `@bookTicker` — Binance USD-M.

use crate::common::*;
use jeed_wire::{Scale, Venue, WireKind, WireRecord, header_flags, quote_ext};
use jeed_crypto::binance::futures::bbo;

#[test]
fn decodes_the_documented_frame() {
    let inst = futures_btcusdt();
    let mut rec = WireRecord::zeroed();
    bbo::decode(&inst, FUT_BOOK_TICKER, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.venue(), Ok(Venue::BinanceFutures));
    assert_eq!(rec.header.qty_scale(), Ok(Scale::S3), "USD-M sizes are three decimals");
    assert_eq!(rec.header.depth, 1);
    assert_eq!(rec.validate(), Ok(()));

    let q = rec.quote().unwrap();
    assert_eq!(q.bid[0].price, 11_925_000);
    assert_eq!(q.bid[0].qty, 31_210);
    assert_eq!(q.ask[0].price, 11_925_010);
    assert_eq!(q.ask[0].qty, 40_660);
    assert_eq!(q.quote_ext_kind, quote_ext::SEQUENCE);
    assert_eq!(q.quote_ext, 400_900_217);
}

#[test]
fn venue_time_is_t_not_e() {
    // `T` is when the book changed, `E` is when the message was pushed. The
    // difference between them is Binance's publishing latency, and taking `E`
    // would fold it into the market's clock.
    let inst = futures_btcusdt();
    let mut rec = WireRecord::zeroed();
    bbo::decode(&inst, FUT_BOOK_TICKER, RECV_NS, &mut rec).unwrap();

    assert!(rec.header.has(header_flags::VENUE_TIME_VALID));
    assert_eq!(rec.header.venue_time(), Some(1_568_014_460_891_000_000));
}

#[test]
fn the_same_symbol_on_the_other_market_is_a_different_instrument() {
    // `BTCUSDT` is a spot pair on one venue byte and a perpetual on the other.
    // If they shared a venue the consumer would merge two books.
    let spot = spot_btcusdt();
    let futures = futures_btcusdt();
    assert_eq!(spot.symbol_bytes(), futures.symbol_bytes());
    assert_ne!(spot.venue(), futures.venue());
}
