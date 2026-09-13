//! `/market/match` — KuCoin spot.

use crate::common::*;
use jeed_crypto::CryptoError;
use jeed_crypto::kucoin::spot::trade;
use jeed_wire::{Venue, WireKind, WireRecord, trade_kind};

#[test]
fn a_print_carries_price_size_and_the_takers_side() {
    let inst = spot_btc_usdt();
    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, SPOT_MATCH, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Trade));
    assert_eq!(rec.header.venue(), Ok(Venue::KucoinSpot));
    assert_eq!(rec.header.symbol_bytes(), b"BTC-USDT");
    assert_eq!(rec.validate(), Ok(()));

    let t = rec.trade().unwrap();
    assert_eq!(t.price, 35_355);
    assert_eq!(t.qty, 1_022_222);
    assert_eq!(t.trade_kind, trade_kind::BUY);
}

#[test]
fn the_nanosecond_clock_is_carried_as_it_arrived() {
    // `time` is already nanoseconds here. Multiplying it by a million, as
    // every millisecond venue's decoder does, would land in the year 56000.
    let inst = spot_btc_usdt();
    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, SPOT_MATCH, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.venue_time(), Some(1_545_913_818_099_321_004));
}

#[test]
fn a_frame_for_another_instrument_is_refused() {
    let inst = spot_btc_usdt();
    let frame = br#"{"type":"message","topic":"/market/match:ETH-USDT","subject":"trade.l3match","data":{"symbol":"ETH-USDT","side":"buy","price":"2000.0","size":"1.00000000","time":"1545913818099321004"}}"#;

    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(trade::decode(&inst, frame, RECV_NS, &mut rec), Err(CryptoError::SymbolMismatch));
    assert_eq!(rec, before);
}

#[test]
fn a_print_with_no_size_leaves_the_record_alone() {
    let inst = spot_btc_usdt();
    let frame = br#"{"type":"message","topic":"/market/match:BTC-USDT","subject":"trade.l3match","data":{"symbol":"BTC-USDT","side":"buy","price":"3535.5","time":"1545913818099321004"}}"#;

    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(
        trade::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "size" })
    );
    assert_eq!(rec, before);
}

#[test]
fn a_size_finer_than_the_scale_is_refused_rather_than_rounded() {
    // Eight decimals is what the instrument was configured with. A ninth that
    // is not zero means the configuration is wrong, and a rounded size is a
    // plausible-looking wrong number.
    let inst = spot_btc_usdt();
    let frame = br#"{"type":"message","topic":"/market/match:BTC-USDT","subject":"trade.l3match","data":{"symbol":"BTC-USDT","side":"sell","price":"3535.5","size":"0.010222225","time":"1545913818099321004"}}"#;

    let mut rec = WireRecord::zeroed();
    assert!(matches!(
        trade::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Field { key: "size", .. })
    ));
}
