//! `jeed_krx::decode::bond::trade` — `A3` 채권 체결.

use crate::common::{BondA3, RECV_NS, VENUE_NS};
use jeed_krx::TrCode as T;
use jeed_krx::KrxError;
use jeed_krx::decode::bond::trade::{DECODER, MESSAGE_LEN, handles};
use jeed_wire::{Scale, WireKind, WireRecord, header_flags, trade_flags, trade_kind};

fn decode(msg: &[u8]) -> WireRecord {
    let mut out = WireRecord::zeroed();
    DECODER.decode(msg, RECV_NS, &mut out).expect("decodes");
    out
}

#[test]
fn the_message_is_the_length_the_spec_defines() {
    assert_eq!(MESSAGE_LEN, 223); // IFMSRPD0027
}

#[test]
fn a_print_round_trips_into_a_trade_record() {
    let rec = decode(&BondA3::ktb().build());

    assert_eq!(rec.validate(), Ok(()));
    assert_eq!(rec.kind(), Ok(WireKind::Trade));
    assert_eq!(rec.header.symbol_bytes(), b"KR103501GA98");
    assert_eq!(rec.header.price_scale(), Ok(Scale::S2));

    let t = rec.trade().unwrap();
    assert_eq!(t.price, 1_034_550, "10,345.50원 at two places");
    assert_eq!(t.qty, 100_000, "천원 단위");
    assert_eq!(t.cumulative_qty(), Some(8_500_000));
}

#[test]
fn a_small_lot_print_is_the_same_interface_under_another_code() {
    // IFMSRPD0027 is one message for all three 채권 markets; only the trcode
    // and the ISIN say which one sent it.
    let rec = decode(&BondA3::nhb().build());
    assert_eq!(rec.kind(), Ok(WireKind::Trade));
    assert_eq!(rec.header.symbol_bytes(), b"KR2001022C74");
    assert_eq!(rec.trade().unwrap().price, 1_034_550);
    assert_eq!(rec.header.price_scale(), Ok(Scale::S2));
}

#[test]
fn the_trade_yield_is_carried_and_marked() {
    let rec = decode(&BondA3::ktb().build());
    let t = rec.trade().unwrap();
    assert_eq!(t.trade_yield(), Some(3_123_456));
    assert!(t.trade_flags & trade_flags::YIELD_VALID != 0);
}

#[test]
fn there_is_no_aggressor_field_at_all_on_this_channel() {
    // 채권 체결 does not say which side took. NONE is "this channel has no such
    // field"; UNKNOWN would say the field was there and said nothing, which is
    // what 증권 and 파생 send for an auction cross.
    let rec = decode(&BondA3::ktb().build());
    let t = rec.trade().unwrap();
    assert_eq!(t.trade_kind, trade_kind::NONE);
    assert_ne!(t.trade_kind, trade_kind::UNKNOWN);
}

#[test]
fn there_is_no_dynamic_band_either() {
    assert_eq!(decode(&BondA3::ktb().build()).trade().unwrap().dyn_limits(), None);
}

#[test]
fn the_exchange_timestamp_is_assembled_and_marked_valid() {
    let rec = decode(&BondA3::ktb().build());
    assert!(rec.header.has(header_flags::VENUE_TIME_VALID));
    assert_eq!(rec.header.venue_ns, VENUE_NS);
}

#[test]
fn a_yield_too_wide_for_the_wire_is_an_error_not_a_wrap() {
    // The field has room for five integer digits; the wire's BookYield is an
    // i32, which at six decimal places tops out around 2147%. No bond trades
    // there, but a corrupt message could say so, and a wrap would put a
    // perfectly plausible yield on the wire.
    let mut msg = BondA3::ktb();
    msg.trade.trade_yield = "099999.999999";

    let mut out = WireRecord::zeroed();
    assert!(DECODER.decode(&msg.build(), RECV_NS, &mut out).is_err());
    assert_eq!(out, WireRecord::zeroed());
}

#[test]
fn a_corrupt_field_leaves_the_output_untouched() {
    let mut msg = BondA3::ktb().build();
    msg[41] = b'X'; // sign byte of 체결가격

    let mut out = WireRecord::zeroed();
    assert!(DECODER.decode(&msg, RECV_NS, &mut out).is_err());
    assert_eq!(out, WireRecord::zeroed(), "no partial update");
}

#[test]
fn a_message_of_the_wrong_length_is_refused() {
    let mut msg = BondA3::ktb().build();
    msg.truncate(220);
    let mut out = WireRecord::zeroed();
    assert_eq!(
        DECODER.decode(&msg, RECV_NS, &mut out),
        Err(KrxError::Length { expected: 223, actual: 220 })
    );
}

#[test]
fn every_bond_market_but_repo_is_claimed() {
    assert!(handles(T::new(*b"A301B")), "일반채권");
    assert!(handles(T::new(*b"A301K")), "국고채권");
    assert!(handles(T::new(*b"A301M")), "소액채권");
    // REPO sends this same interface, but its price shape differs and its
    // quote forms are not decoded — a half-covered market is worse than an
    // uncovered one.
    assert!(!handles(T::new(*b"A301R")), "REPO");
    assert!(!handles(T::new(*b"A301F")), "파생");
}
