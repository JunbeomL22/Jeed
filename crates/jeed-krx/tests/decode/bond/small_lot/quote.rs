//! `jeed_krx::decode::bond::small_lot::quote` — `B6` 소액채권 우선호가.

use crate::common::{RECV_NS, SmallLotB6, SmallLotLevel, VENUE_NS, nhb_book};
use jeed_krx::KrxError;
use jeed_krx::TrCode as T;
use jeed_krx::decode::bond::small_lot::quote::{DECODER, MESSAGE_LEN, handles};
use jeed_wire::{Scale, WireKind, WireRecord, header_flags, level_ext};

fn decode(msg: &[u8]) -> WireRecord {
    let mut out = WireRecord::zeroed();
    DECODER.decode(msg, RECV_NS, &mut out).expect("decodes");
    out
}

#[test]
fn the_message_is_the_length_the_spec_defines() {
    assert_eq!(MESSAGE_LEN, 882); // IFMSRPD0024
    assert_eq!(DECODER.depth(), 5);
}

#[test]
fn a_national_housing_bond_book_round_trips_into_a_quote_record() {
    let rec = decode(&SmallLotB6::nhb(nhb_book()).build());

    assert_eq!(rec.validate(), Ok(()));
    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.symbol_bytes(), b"KR2001022C74");
    assert_eq!(rec.header.depth, 3);
    assert_eq!(rec.header.price_scale(), Ok(Scale::S2));

    let q = rec.quote().unwrap();
    assert_eq!(q.ask[0].price, 1_034_550, "10,345.50원 at two places");
    assert_eq!(q.bid[0].price, 1_034_000);
    assert_eq!(q.ask[0].qty, 500_000);
    assert_eq!(q.bid[0].qty, 300_000);
}

#[test]
fn the_instruments_own_block_is_read_and_the_class_block_is_not() {
    // Each level is the instrument's six fields followed by the 채권종류
    // aggregate's six. The builder puts the aggregate at a different price
    // and a far larger size, so reading the wrong block changes every value.
    let rec = decode(&SmallLotB6::nhb(nhb_book()).build());
    let q = rec.quote().unwrap();

    assert_eq!(q.ask[0].price, 1_034_550, "own, not the class's 10,300.00");
    assert_eq!(q.ask[0].qty, 500_000, "own, not the class's 9,000,000");
    assert_eq!(q.ask[0].bond_yield(), 3_123_456, "own, not the class's 3.20%");
}

#[test]
fn the_stride_is_one_hundred_fifty_six_bytes_not_seventy_eight() {
    // Stepping 78 bytes would land the second level on the first level's
    // 채권종류 block — a parseable, plausible, wrong book.
    let rec = decode(&SmallLotB6::nhb(nhb_book()).build());
    let q = rec.quote().unwrap();

    assert_eq!(q.ask[1].price, 1_035_000, "level 2 own, not level 1 class (10,300.00)");
    assert_eq!(q.ask[1].qty, 700_000);
    assert_eq!(q.ask[2].price, 1_035_500);
    assert_eq!(q.bid[2].price, 1_033_000);
}

#[test]
fn the_forty_one_byte_header_is_the_bond_one() {
    let rec = decode(&SmallLotB6::nhb(nhb_book()).build());
    assert!(rec.header.has(header_flags::VENUE_TIME_VALID));
    assert_eq!(rec.header.venue_ns, VENUE_NS);
}

#[test]
fn every_level_carries_its_yield() {
    let rec = decode(&SmallLotB6::nhb(nhb_book()).build());
    let q = rec.quote().unwrap();
    assert_eq!(q.level_ext_kind, level_ext::BOND_YIELD);
    assert_eq!(q.ask[0].bond_yield(), 3_123_456);
    assert_eq!(q.bid[0].bond_yield(), 3_128_900);
    assert!(q.ask[1].bond_yield() < q.ask[0].bond_yield(), "yield runs against price");
}

#[test]
fn quantities_are_thousand_won_face_and_the_scale_does_not_say_so() {
    let rec = decode(&SmallLotB6::nhb(nhb_book()).build());
    assert_eq!(rec.header.qty_scale(), Ok(Scale::S0));
}

#[test]
fn depth_is_counted_not_assumed() {
    assert_eq!(decode(&SmallLotB6::nhb(nhb_book()).build()).header.depth, 3);
    let empty = decode(&SmallLotB6::nhb(vec![SmallLotLevel::EMPTY; 5]).build());
    assert_eq!(empty.header.depth, 0);
    assert!(empty.header.has(header_flags::BID_EMPTY));
    assert!(empty.header.has(header_flags::ASK_EMPTY));
}

#[test]
fn a_class_only_book_is_an_empty_book() {
    // The 채권종류 aggregate can rest where the instrument itself has nothing.
    // That aggregate is not this instrument's book, so the record says empty
    // rather than borrowing the class's levels.
    let mut levels = nhb_book();
    for l in &mut levels {
        l.own = crate::common::BondLevel::EMPTY;
    }
    let rec = decode(&SmallLotB6::nhb(levels).build());
    assert_eq!(rec.header.depth, 0);
    assert!(rec.header.has(header_flags::BID_EMPTY));
    assert!(rec.header.has(header_flags::ASK_EMPTY));
}

#[test]
fn a_corrupt_field_leaves_the_output_untouched() {
    let mut msg = SmallLotB6::nhb(nhb_book()).build();
    msg[41] = b'X'; // sign byte of the first ask price

    let mut out = WireRecord::zeroed();
    assert!(DECODER.decode(&msg, RECV_NS, &mut out).is_err());
    assert_eq!(out, WireRecord::zeroed(), "no partial update");
}

#[test]
fn a_general_bond_message_behind_this_code_is_refused_by_length() {
    // The frame check runs first: a 462 B 일반채권 book relabelled `B601M`
    // never reaches a field reader.
    let mut msg = crate::common::BondB6::ktb(crate::common::ktb_book()).build();
    msg[..5].copy_from_slice(b"B601M");
    let mut out = WireRecord::zeroed();
    assert_eq!(
        DECODER.decode(&msg, RECV_NS, &mut out),
        Err(KrxError::Length { expected: 882, actual: 462 })
    );
    assert_eq!(out, WireRecord::zeroed());
}

#[test]
fn only_the_small_lot_market_is_claimed() {
    assert!(handles(T::new(*b"B601M")), "소액채권");
    assert!(!handles(T::new(*b"B601B")), "일반채권 is IFMSRPD0023");
    assert!(!handles(T::new(*b"B601K")), "국고채권 is IFMSRPD0023");
    assert!(!handles(T::new(*b"B601R")), "REPO is IFMSRPD0025");
    assert!(!handles(T::new(*b"G701M")), "wrong data class");
}
