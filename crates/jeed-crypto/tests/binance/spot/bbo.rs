//! `@bookTicker` — Binance spot.

use crate::common::*;
use jeed_crypto::binance::spot::bbo;
use jeed_crypto::{CryptoError, Instrument};
use jeed_convert::ParseErr;
use jeed_wire::{Scale, Venue, WireKind, WireRecord, header_flags, quote_ext};

#[test]
fn decodes_the_documented_frame() {
    let inst = spot_btcusdt();
    let mut rec = WireRecord::zeroed();
    bbo::decode(&inst, SPOT_BOOK_TICKER, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.venue(), Ok(Venue::BinanceSpot));
    assert_eq!(rec.header.symbol_bytes(), b"BTCUSDT");
    assert_eq!(rec.header.recv_ns, RECV_NS);
    assert_eq!(rec.header.price_scale(), Ok(Scale::S2));
    assert_eq!(rec.header.qty_scale(), Ok(Scale::S5));
    assert_eq!(rec.header.depth, 1);
    assert_eq!(rec.validate(), Ok(()));

    let q = rec.quote().unwrap();
    // 119250.01 at two decimals is 11_925_001; 3.121 at five is 312_100.
    assert_eq!(q.bid[0].price, 11_925_001);
    assert_eq!(q.bid[0].qty, 312_100);
    assert_eq!(q.ask[0].price, 11_925_002);
    assert_eq!(q.ask[0].qty, 406_600);
    assert_eq!(q.quote_ext_kind, quote_ext::SEQUENCE);
    assert_eq!(q.quote_ext, 400_900_217);
}

#[test]
fn spot_book_ticker_has_no_venue_time() {
    // USD-M's has `T`; this one has no clock field at all. A zero that claimed
    // to be a timestamp would make every quote look decades stale.
    let inst = spot_btcusdt();
    let mut rec = WireRecord::zeroed();
    bbo::decode(&inst, SPOT_BOOK_TICKER, RECV_NS, &mut rec).unwrap();

    assert!(!rec.header.has(header_flags::VENUE_TIME_VALID));
    assert_eq!(rec.header.venue_time(), None);
    assert_eq!(rec.header.venue_ns, 0);
}

#[test]
fn an_empty_side_is_flagged_and_not_counted() {
    let inst = spot_btcusdt();
    let frame = br#"{"u":1,"s":"BTCUSDT","b":"25.35","B":"0","a":"25.36","A":"40.66000"}"#;
    let mut rec = WireRecord::zeroed();
    bbo::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    assert!(rec.header.has(header_flags::BID_EMPTY));
    assert!(!rec.header.has(header_flags::ASK_EMPTY));
    assert_eq!(rec.header.depth, 1, "the ask side still has a level");
}

#[test]
fn a_frame_for_another_instrument_is_refused() {
    // One decoder is one subscription, so this is a wiring mistake. Left
    // unchecked it would publish ETH prices under BTCUSDT.
    let inst = spot_btcusdt();
    let frame = br#"{"u":1,"s":"ETHUSDT","b":"25.35","B":"1.00000","a":"25.36","A":"1.00000"}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(bbo::decode(&inst, frame, RECV_NS, &mut rec), Err(CryptoError::SymbolMismatch));
    assert_eq!(rec, before, "a refused frame leaves the slot alone");
}

#[test]
fn lower_case_symbol_is_refused() {
    // The stream URL is lower-case and the `s` field is upper-case. A decoder
    // configured from the URL should find out loudly.
    let inst = Instrument::new(Venue::BinanceSpot, b"btcusdt", 2, 5).unwrap();
    let mut rec = WireRecord::zeroed();
    assert_eq!(
        bbo::decode(&inst, SPOT_BOOK_TICKER, RECV_NS, &mut rec),
        Err(CryptoError::SymbolMismatch)
    );
}


#[test]
fn a_missing_field_is_named() {
    let inst = spot_btcusdt();
    let frame = br#"{"u":1,"s":"BTCUSDT","b":"25.35","a":"25.36","A":"40.66000"}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(bbo::decode(&inst, frame, RECV_NS, &mut rec), Err(CryptoError::Missing { key: "B" }));
    assert_eq!(rec, before);
}

#[test]
fn a_size_finer_than_the_scale_fails_rather_than_rounds() {
    // Five decimals of size configured, six sent. That means the instrument's
    // stepSize changed — dropping the frame is right; publishing a rounded
    // size would be a plausible-looking wrong number.
    let inst = spot_btcusdt();
    let frame = br#"{"u":1,"s":"BTCUSDT","b":"25.35","B":"0.000001","a":"25.36","A":"40.66000"}"#;
    let mut rec = dirty_record();
    let before = rec;

    assert_eq!(
        bbo::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Field { key: "B", err: ParseErr::Precision })
    );
    assert_eq!(rec, before);
}

#[test]
fn trailing_zeros_past_the_scale_are_fine() {
    // Binance pads every number to the symbol's full precision, so this is the
    // normal case, not the exception.
    let inst = spot_btcusdt();
    let frame = br#"{"u":1,"s":"BTCUSDT","b":"25.35000000","B":"1.00000000","a":"25.36000000","A":"1.00000000"}"#;
    let mut rec = WireRecord::zeroed();
    bbo::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    let q = rec.quote().unwrap();
    assert_eq!(q.bid[0].price, 2_535);
    assert_eq!(q.bid[0].qty, 100_000);
}
