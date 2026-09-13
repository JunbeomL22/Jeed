//! `jeed_crypto::bithumb` — Upbit's decoders, a different exchange.
//!
//! There is no Bithumb decoder to test: the module re-exports Upbit's, because
//! Bithumb's public v2 WebSocket API is Upbit's field for field. What is worth
//! testing is the part that is *not* shared — the venue byte, which is the
//! whole of the difference between a Bithumb record and an Upbit one and comes
//! from the instrument rather than from the module path.

use jeed_crypto::{Instrument, bithumb, upbit};
use jeed_wire::{Venue, WireKind, WireRecord};

const RECV_NS: u64 = 1_704_067_300_000_000_000;

const ORDERBOOK: &[u8] = br#"{"type":"orderbook","code":"KRW-BTC","timestamp":1704067200000,"orderbook_units":[{"ask_price":152430000.0,"bid_price":152400000.0,"ask_size":0.17,"bid_size":0.23}]}"#;

const TRADE: &[u8] = br#"{"type":"trade","code":"KRW-BTC","trade_timestamp":1704067200000,"trade_price":152430000.0,"trade_volume":0.00084280,"ask_bid":"BID"}"#;

fn bithumb_krw_btc() -> Instrument {
    Instrument::new(Venue::Bithumb, b"KRW-BTC", 0, 8).expect("valid instrument")
}

fn upbit_krw_btc() -> Instrument {
    Instrument::new(Venue::Upbit, b"KRW-BTC", 0, 8).expect("valid instrument")
}

#[test]
fn a_bithumb_book_carries_the_bithumb_venue() {
    let mut rec = WireRecord::zeroed();
    bithumb::snapshot::decode(&bithumb_krw_btc(), ORDERBOOK, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.venue(), Ok(Venue::Bithumb));
    assert_eq!(rec.header.symbol_bytes(), b"KRW-BTC");
    assert_eq!(rec.quote().unwrap().ask[0].price, 152_430_000);
}

#[test]
fn a_bithumb_print_carries_the_bithumb_venue() {
    let mut rec = WireRecord::zeroed();
    bithumb::trade::decode(&bithumb_krw_btc(), TRADE, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Trade));
    assert_eq!(rec.header.venue(), Ok(Venue::Bithumb));
    assert_eq!(rec.trade().unwrap().qty, 84_280);
}

#[test]
fn the_two_exchanges_differ_by_exactly_one_byte() {
    // `KRW-BTC` is a market on both, and the two books have different prices
    // in them. Identity on the wire is `(venue, symbol)`, so the venue byte is
    // the only thing keeping a consumer from merging them.
    let mut a = WireRecord::zeroed();
    bithumb::snapshot::decode(&bithumb_krw_btc(), ORDERBOOK, RECV_NS, &mut a).unwrap();

    let mut b = WireRecord::zeroed();
    upbit::snapshot::decode(&upbit_krw_btc(), ORDERBOOK, RECV_NS, &mut b).unwrap();

    assert_ne!(a, b);
    assert_ne!(a.header.venue, b.header.venue);

    a.header.venue = b.header.venue;
    assert_eq!(a, b, "everything else about the two records is identical");
}

#[test]
fn the_module_path_is_documentation_and_the_instrument_is_the_truth() {
    // Handing an Upbit instrument to `bithumb::` produces an Upbit record.
    // Worth pinning: here the instrument is the *only* thing separating the
    // two exchanges, so a wiring mistake has nothing else to trip over.
    let mut rec = WireRecord::zeroed();
    bithumb::trade::decode(&upbit_krw_btc(), TRADE, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.venue(), Ok(Venue::Upbit));
}
