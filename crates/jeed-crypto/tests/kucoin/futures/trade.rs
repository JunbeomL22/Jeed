//! `/contractMarket/execution` — KuCoin futures.

use crate::common::*;
use jeed_crypto::CryptoError;
use jeed_crypto::kucoin::futures::trade;
use jeed_wire::{Scale, Venue, WireKind, WireRecord, trade_kind};

#[test]
fn a_print_carries_contracts_and_the_takers_side() {
    let inst = futures_xbtusdtm();
    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, FUTURES_EXECUTION, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::Trade));
    assert_eq!(rec.header.venue(), Ok(Venue::KucoinFutures));
    assert_eq!(rec.header.symbol_bytes(), b"XBTUSDTM");
    assert_eq!(rec.header.qty_scale(), Ok(Scale::S0));
    assert_eq!(rec.validate(), Ok(()));

    let t = rec.trade().unwrap();
    assert_eq!(t.price, 721_373);
    assert_eq!(t.qty, 5, "five contracts, not five of anything else");
    assert_eq!(t.trade_kind, trade_kind::BUY);
}

#[test]
fn the_bare_nanosecond_ts_is_carried_as_it_arrived() {
    let inst = futures_xbtusdtm();
    let mut rec = WireRecord::zeroed();
    trade::decode(&inst, FUTURES_EXECUTION, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.header.venue_time(), Some(1_712_570_590_399_000_000));
}

#[test]
fn a_frame_for_another_instrument_is_refused() {
    let inst = futures_xbtusdtm();
    let frame = br#"{"topic":"/contractMarket/execution:ETHUSDTM","type":"message","subject":"match","data":{"symbol":"ETHUSDTM","side":"sell","size":2,"price":"3000.0","ts":1712570590399000000}}"#;

    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(trade::decode(&inst, frame, RECV_NS, &mut rec), Err(CryptoError::SymbolMismatch));
    assert_eq!(rec, before);
}

#[test]
fn a_print_with_no_price_leaves_the_record_alone() {
    let inst = futures_xbtusdtm();
    let frame = br#"{"topic":"/contractMarket/execution:XBTUSDTM","type":"message","subject":"match","data":{"symbol":"XBTUSDTM","side":"buy","size":5,"ts":1712570590399000000}}"#;

    let mut rec = dirty_record();
    let before = rec;
    assert_eq!(
        trade::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Missing { key: "price" })
    );
    assert_eq!(rec, before);
}

#[test]
fn a_fractional_contract_count_is_refused() {
    // The instrument says zero decimals because contracts are whole. A size
    // with a fraction means the venue changed, not that it should be rounded.
    let inst = futures_xbtusdtm();
    let frame = br#"{"topic":"/contractMarket/execution:XBTUSDTM","type":"message","subject":"match","data":{"symbol":"XBTUSDTM","side":"buy","size":5.5,"price":"72137.3","ts":1712570590399000000}}"#;

    let mut rec = WireRecord::zeroed();
    assert!(matches!(
        trade::decode(&inst, frame, RECV_NS, &mut rec),
        Err(CryptoError::Field { key: "size", .. })
    ));
}
