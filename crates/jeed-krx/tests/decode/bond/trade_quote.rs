//! `jeed_krx::decode::bond::trade_quote` — `G7` 채권 체결 + 우선호가.

use crate::common::{BondA3, BondB6, BondG7, RECV_NS, ktb_book};
use jeed_krx::decode::bond::trade_quote::{DECODER, MESSAGE_LEN, handles};
use jeed_krx::decode::bond::{quote, trade};
use jeed_wire::{Scale, WireKind, WireRecord, level_ext};

fn decode(msg: &[u8]) -> WireRecord {
    let mut out = WireRecord::zeroed();
    DECODER.decode(msg, RECV_NS, &mut out).expect("decodes");
    out
}

#[test]
fn the_message_is_the_length_the_spec_defines() {
    assert_eq!(MESSAGE_LEN, 643); // IFMSRPD0029
    assert_eq!(DECODER.depth(), 5);
}

#[test]
fn the_print_and_the_book_it_left_arrive_as_one_record() {
    let rec = decode(&BondG7::ktb(ktb_book()).build());

    assert_eq!(rec.validate(), Ok(()));
    assert_eq!(rec.kind(), Ok(WireKind::TradeQuote));
    assert_eq!(rec.header.price_scale(), Ok(Scale::S0));

    let tq = rec.trade_quote().unwrap();
    assert_eq!(tq.trade.price, 10345);
    assert_eq!(tq.trade.trade_yield(), Some(3_123_456));
    assert_eq!(tq.quote.ask[0].price, 10345);
    assert_eq!(tq.quote.bid[0].price, 10340);
    assert_eq!(rec.header.depth, 3);
}

#[test]
fn it_is_the_a3_trade_block_and_the_b6_book_verbatim() {
    // G7 is A3's body followed by B6's levels, which is why each is read by one
    // function used from both. If these ever disagree, one copy has drifted.
    let combined = decode(&BondG7::ktb(ktb_book()).build());
    let tq = combined.trade_quote().unwrap();

    let mut only_trade = WireRecord::zeroed();
    trade::DECODER.decode(&BondA3::ktb().build(), RECV_NS, &mut only_trade).expect("decodes");
    assert_eq!(tq.trade, *only_trade.trade().unwrap());

    let mut only_quote = WireRecord::zeroed();
    quote::DECODER
        .decode(&BondB6::ktb(ktb_book()).build(), RECV_NS, &mut only_quote)
        .expect("decodes");
    assert_eq!(tq.quote, *only_quote.quote().unwrap());
    assert_eq!(combined.header.depth, only_quote.header.depth);
}

#[test]
fn the_book_keeps_its_yields() {
    let rec = decode(&BondG7::ktb(ktb_book()).build());
    let q = rec.trade_quote().unwrap().quote;
    assert_eq!(q.level_ext_kind, level_ext::BOND_YIELD);
    assert_eq!(q.ask[0].bond_yield(), 3_123_456);
}

#[test]
fn a_corrupt_field_leaves_the_output_untouched() {
    let mut msg = BondG7::ktb(ktb_book()).build();
    msg[222] = b'X'; // sign byte of the first ask price, just past the trade block

    let mut out = WireRecord::zeroed();
    assert!(DECODER.decode(&msg, RECV_NS, &mut out).is_err());
    assert_eq!(out, WireRecord::zeroed(), "no partial update");
}

#[test]
fn only_general_and_government_bonds_are_claimed() {
    use jeed_krx::TrCode as T;
    assert!(handles(T::new(*b"G701B")));
    assert!(handles(T::new(*b"G701K")));
    assert!(!handles(T::new(*b"G701M")), "소액채권 is IFMSRPD0030");
    assert!(!handles(T::new(*b"G701R")), "REPO is IFMSRPD0031");
    assert!(!handles(T::new(*b"G701F")), "파생");
}
