//! `jeed_krx::decode::bond::small_lot::trade_quote` — `G7` 소액채권 체결 + 우선호가.

use crate::common::{BondA3, RECV_NS, SmallLotB6, SmallLotG7, nhb_book};
use jeed_krx::KrxError;
use jeed_krx::TrCode as T;
use jeed_krx::decode::bond::small_lot::quote;
use jeed_krx::decode::bond::small_lot::trade_quote::{DECODER, MESSAGE_LEN, handles};
use jeed_krx::decode::bond::trade;
use jeed_wire::{Scale, WireKind, WireRecord, level_ext};

fn decode(msg: &[u8]) -> WireRecord {
    let mut out = WireRecord::zeroed();
    DECODER.decode(msg, RECV_NS, &mut out).expect("decodes");
    out
}

#[test]
fn the_message_is_the_length_the_spec_defines() {
    assert_eq!(MESSAGE_LEN, 1063); // IFMSRPD0030
    assert_eq!(DECODER.depth(), 5);
}

#[test]
fn the_print_and_the_book_it_left_arrive_as_one_record() {
    let rec = decode(&SmallLotG7::nhb(nhb_book()).build());

    assert_eq!(rec.validate(), Ok(()));
    assert_eq!(rec.kind(), Ok(WireKind::TradeQuote));
    assert_eq!(rec.header.symbol_bytes(), b"KR2001022C74");
    assert_eq!(rec.header.price_scale(), Ok(Scale::S2));

    let tq = rec.trade_quote().unwrap();
    assert_eq!(tq.trade.price, 1_034_550);
    assert_eq!(tq.trade.trade_yield(), Some(3_123_456));
    assert_eq!(tq.quote.ask[0].price, 1_034_550);
    assert_eq!(tq.quote.bid[0].price, 1_034_000);
    assert_eq!(tq.quote.ask[1].price, 1_035_000, "stride 156, not 78");
    assert_eq!(rec.header.depth, 3);
}

#[test]
fn it_is_the_a3_trade_block_and_the_small_lot_b6_book_verbatim() {
    // G7 is A3's body followed by IFMSRPD0024's levels, each read by one
    // function used from both. If these ever disagree, one copy has drifted.
    let combined = decode(&SmallLotG7::nhb(nhb_book()).build());
    let tq = combined.trade_quote().unwrap();

    let mut only_trade = WireRecord::zeroed();
    trade::DECODER.decode(&BondA3::nhb().build(), RECV_NS, &mut only_trade).expect("decodes");
    assert_eq!(tq.trade, *only_trade.trade().unwrap());

    let mut only_quote = WireRecord::zeroed();
    quote::DECODER
        .decode(&SmallLotB6::nhb(nhb_book()).build(), RECV_NS, &mut only_quote)
        .expect("decodes");
    assert_eq!(tq.quote, *only_quote.quote().unwrap());
    assert_eq!(combined.header.depth, only_quote.header.depth);
}

#[test]
fn the_book_keeps_its_yields() {
    let rec = decode(&SmallLotG7::nhb(nhb_book()).build());
    let q = rec.trade_quote().unwrap().quote;
    assert_eq!(q.level_ext_kind, level_ext::BOND_YIELD);
    assert_eq!(q.ask[0].bond_yield(), 3_123_456);
}

#[test]
fn a_corrupt_field_leaves_the_output_untouched() {
    let mut msg = SmallLotG7::nhb(nhb_book()).build();
    msg[222] = b'X'; // sign byte of the first ask price, just past the trade block

    let mut out = WireRecord::zeroed();
    assert!(DECODER.decode(&msg, RECV_NS, &mut out).is_err());
    assert_eq!(out, WireRecord::zeroed(), "no partial update");
}

#[test]
fn a_message_of_the_wrong_length_is_refused() {
    let mut msg = SmallLotG7::nhb(nhb_book()).build();
    msg.truncate(643); // the 일반채권 G7 length
    let mut out = WireRecord::zeroed();
    assert_eq!(
        DECODER.decode(&msg, RECV_NS, &mut out),
        Err(KrxError::Length { expected: 1063, actual: 643 })
    );
}

#[test]
fn only_the_small_lot_market_is_claimed() {
    assert!(handles(T::new(*b"G701M")), "소액채권");
    assert!(!handles(T::new(*b"G701B")), "일반채권 is IFMSRPD0029");
    assert!(!handles(T::new(*b"G701K")), "국고채권 is IFMSRPD0029");
    assert!(!handles(T::new(*b"G701R")), "REPO is IFMSRPD0031");
    assert!(!handles(T::new(*b"B601M")), "wrong data class");
}
