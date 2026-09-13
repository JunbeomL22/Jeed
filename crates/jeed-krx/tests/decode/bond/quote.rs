//! `jeed_krx::decode::bond::quote` — `B6` 일반채권·국고채권 우선호가.

use crate::common::{BondB6, BondLevel, RECV_NS, VENUE_NS, ktb_book};
use jeed_krx::KrxError;
use jeed_krx::decode::bond::quote::{DECODER, MESSAGE_LEN, handles};
use jeed_wire::{Scale, WireKind, WireRecord, header_flags, level_ext};

fn decode(msg: &[u8]) -> WireRecord {
    let mut out = WireRecord::zeroed();
    DECODER.decode(msg, RECV_NS, &mut out).expect("decodes");
    out
}

#[test]
fn the_message_is_the_length_the_spec_defines() {
    assert_eq!(MESSAGE_LEN, 462); // IFMSRPD0023
    assert_eq!(DECODER.depth(), 5);
}

#[test]
fn a_ktb_book_round_trips_into_a_quote_record() {
    let rec = decode(&BondB6::ktb(ktb_book()).build());

    assert_eq!(rec.validate(), Ok(()));
    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.symbol_bytes(), b"KR103501GA98");
    assert_eq!(rec.header.depth, 3);
    assert_eq!(rec.header.price_scale(), Ok(Scale::S0));

    let q = rec.quote().unwrap();
    assert_eq!(q.ask[0].price, 10345);
    assert_eq!(q.bid[0].price, 10340);
    assert_eq!(q.ask[0].qty, 500_000);
    assert_eq!(q.bid[0].qty, 300_000);
}

#[test]
fn the_forty_one_byte_header_puts_the_clock_six_bytes_earlier() {
    // 채권 carries no 정보분배종목인덱스. Reading this with the 47-byte reader
    // would take the ISIN correctly and then find the clock inside the first
    // price — which parses, and gives a wrong time and a wrong book.
    let rec = decode(&BondB6::ktb(ktb_book()).build());
    assert!(rec.header.has(header_flags::VENUE_TIME_VALID));
    assert_eq!(rec.header.venue_ns, VENUE_NS);
}

#[test]
fn every_level_carries_its_yield() {
    // Price and yield are two views of the same quote and KRX sends both, so
    // the yield rides in the level's ext word rather than being recomputed
    // downstream from a price and a day count.
    let rec = decode(&BondB6::ktb(ktb_book()).build());
    let q = rec.quote().unwrap();

    assert_eq!(q.level_ext_kind, level_ext::BOND_YIELD);
    assert_eq!(q.ask[0].bond_yield(), 3_123_456, "3.123456% at six places");
    assert_eq!(q.bid[0].bond_yield(), 3_128_900);
}

#[test]
fn the_yield_runs_the_other_way_from_the_price() {
    // A higher ask price is a lower ask yield. A book that got this backwards
    // would pass every structural check and still be impossible.
    let rec = decode(&BondB6::ktb(ktb_book()).build());
    let q = rec.quote().unwrap();

    assert!(q.ask[1].price > q.ask[0].price);
    assert!(q.ask[1].bond_yield() < q.ask[0].bond_yield());
    assert!(q.bid[1].price < q.bid[0].price);
    assert!(q.bid[1].bond_yield() > q.bid[0].bond_yield());
}

#[test]
fn the_ask_yield_sits_below_the_bid_yield() {
    let rec = decode(&BondB6::ktb(ktb_book()).build());
    let q = rec.quote().unwrap();
    assert!(q.ask[0].bond_yield() < q.bid[0].bond_yield());
    assert!(q.ask[0].price > q.bid[0].price);
}

#[test]
fn quantities_are_thousand_won_face_and_the_scale_does_not_say_so() {
    // 천원 is a unit, not a scale. The wire reports what the exchange sent; a
    // consumer comparing this against a share count without knowing that is
    // comparing face value to shares.
    let rec = decode(&BondB6::ktb(ktb_book()).build());
    assert_eq!(rec.header.qty_scale(), Ok(Scale::S0));
    assert_eq!(rec.quote().unwrap().ask[0].qty, 500_000);
}

#[test]
fn order_counts_are_not_claimed() {
    assert!(!decode(&BondB6::ktb(ktb_book()).build()).quote().unwrap().has_order_counts());
}

#[test]
fn depth_is_counted_not_assumed() {
    // The builder's book is three deep in a five-deep message.
    assert_eq!(decode(&BondB6::ktb(ktb_book()).build()).header.depth, 3);
    assert_eq!(decode(&BondB6::ktb(vec![BondLevel::EMPTY; 5]).build()).header.depth, 0);
}

#[test]
fn an_entirely_empty_book_flags_both_sides() {
    let rec = decode(&BondB6::ktb(vec![BondLevel::EMPTY; 5]).build());
    assert!(rec.header.has(header_flags::BID_EMPTY));
    assert!(rec.header.has(header_flags::ASK_EMPTY));
}

#[test]
fn a_blank_sequence_does_not_stop_the_decode() {
    let mut msg = BondB6::ktb(ktb_book());
    msg.header.sequence = None;
    assert_eq!(decode(&msg.build()).kind(), Ok(WireKind::Quote));
}

#[test]
fn a_corrupt_field_leaves_the_output_untouched() {
    let mut msg = BondB6::ktb(ktb_book()).build();
    msg[41] = b'X'; // sign byte of the first ask price

    let mut out = WireRecord::zeroed();
    assert!(DECODER.decode(&msg, RECV_NS, &mut out).is_err());
    assert_eq!(out, WireRecord::zeroed(), "no partial update");
}

#[test]
fn a_message_of_the_wrong_length_is_refused() {
    let mut msg = BondB6::ktb(ktb_book()).build();
    msg.truncate(450);
    let mut out = WireRecord::zeroed();
    assert_eq!(
        DECODER.decode(&msg, RECV_NS, &mut out),
        Err(KrxError::Length { expected: 462, actual: 450 })
    );
}

#[test]
fn only_general_and_government_bonds_are_claimed() {
    use jeed_krx::TrCode as T;
    assert!(handles(T::new(*b"B601B")), "일반채권");
    assert!(handles(T::new(*b"B601K")), "국고채권");
    // 소액채권 and REPO are much larger interfaces this build does not decode.
    assert!(!handles(T::new(*b"B601M")), "소액채권 is IFMSRPD0024");
    assert!(!handles(T::new(*b"B601R")), "REPO is IFMSRPD0025");
    assert!(!handles(T::new(*b"B601S")), "증권");
}
