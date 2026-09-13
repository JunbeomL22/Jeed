//! `jeed_crypto::instrument` — what a decoder is pinned to.

use jeed_convert::ParseErr;
use jeed_crypto::error::InstrumentError;
use jeed_crypto::{CryptoError, Instrument};
use jeed_wire::{Scale, Venue, WireKind};

fn btcusdt() -> Instrument {
    Instrument::new(Venue::BinanceSpot, b"BTCUSDT", 2, 5).unwrap()
}

#[test]
fn the_symbol_reaches_the_header_as_the_venue_spelled_it() {
    // Not normalised to `BTC_USDT`: normalising is a mapping, and a mapping is
    // the consumer's (feed_handler.md §6).
    let h = btcusdt().header(WireKind::Trade, 42);
    assert_eq!(h.symbol_bytes(), b"BTCUSDT");
    assert_eq!(h.venue(), Ok(Venue::BinanceSpot));
    assert_eq!(h.recv_ns, 42);
    assert_eq!(h.price_scale(), Ok(Scale::S2));
    assert_eq!(h.qty_scale(), Ok(Scale::S5));
}

#[test]
fn a_symbol_a_twelve_byte_field_could_not_have_carried() {
    // The reason the wire's identity field is twenty-four bytes.
    let inst = Instrument::new(Venue::BinanceFutures, b"1000000MOGUSDT", 8, 0).unwrap();
    assert_eq!(inst.symbol_bytes(), b"1000000MOGUSDT");
    assert_eq!(inst.header(WireKind::Trade, 0).symbol_bytes(), b"1000000MOGUSDT");
}

#[test]
fn a_symbol_that_will_not_fit_is_refused_at_construction() {
    // At start-up, where it can be fixed, rather than per frame.
    assert_eq!(
        Instrument::new(Venue::BinanceSpot, &[b'X'; 25], 2, 5),
        Err(CryptoError::Instrument(InstrumentError::Symbol))
    );
    assert_eq!(
        Instrument::new(Venue::BinanceSpot, b"", 2, 5),
        Err(CryptoError::Instrument(InstrumentError::Symbol))
    );
}

#[test]
fn a_scale_the_wire_cannot_encode_is_refused() {
    assert_eq!(
        Instrument::new(Venue::BinanceSpot, b"BTCUSDT", 9, 5),
        Err(CryptoError::Instrument(InstrumentError::Scale { decimals: 9 }))
    );
}

#[test]
fn prices_and_sizes_read_on_their_own_scales() {
    let inst = btcusdt();
    assert_eq!(inst.price(b"119250.01000000", "p"), Ok(11_925_001));
    assert_eq!(inst.qty(b"3.12100000", "q"), Ok(312_100));
}

#[test]
fn a_value_finer_than_its_scale_fails_rather_than_rounds() {
    // `to_i64` would answer 11_925_001 and lose the rest silently. That is the
    // right behaviour for a fixed-width KRX field and the wrong one here,
    // where the venue chooses the precision.
    let inst = btcusdt();
    assert_eq!(
        inst.price(b"119250.015", "p"),
        Err(CryptoError::Field { key: "p", err: ParseErr::Precision })
    );
}

#[test]
fn symbol_matching_is_byte_for_byte() {
    let inst = btcusdt();
    assert!(inst.matches(b"BTCUSDT"));
    assert!(!inst.matches(b"btcusdt"), "the `s` field is upper-case");
    assert!(!inst.matches(b"BTCUSD"));
    assert!(!inst.matches(b"BTCUSDTT"), "a prefix is not a match");
    assert_eq!(inst.check_symbol(b"ETHUSDT"), Err(CryptoError::SymbolMismatch));
}

#[test]
fn exponent_form_reads_as_the_same_number() {
    // Upbit sends KRW prices above ten million as `1.04525E8`.
    let inst = Instrument::new(Venue::Upbit, b"KRW-BTC", 0, 8).unwrap();
    assert_eq!(inst.price(b"1.04525E8", "trade_price").unwrap(), 104_525_000);
    assert_eq!(inst.price(b"104525000.0", "trade_price").unwrap(), 104_525_000);
    assert_eq!(inst.qty(b"8.428E-4", "trade_volume").unwrap(), 84_280);
    assert_eq!(inst.qty(b"0.00084280", "trade_volume").unwrap(), 84_280);

    // The scale still guards what the exponent unfolds to.
    let e = inst.price(b"1.045251E2", "trade_price").unwrap_err();
    assert_eq!(e, CryptoError::Field { key: "trade_price", err: ParseErr::Precision });
    let e = inst.price(b"1.5E", "trade_price").unwrap_err();
    assert_eq!(e, CryptoError::Field { key: "trade_price", err: ParseErr::InvalidDigit });
}
