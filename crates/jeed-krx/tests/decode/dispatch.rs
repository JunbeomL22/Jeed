//! `jeed_krx::decode::dispatch` — picking a decoder from the five trcode bytes.

use crate::common::{
    A3, B6, BondA3, BondB6, BondG7, EquityA3, EtfB7, G7, M4, Q2, RECV_NS, StockB6, V1, ktb_book,
    kospi200_book, single_stock_book,
};
use jeed_krx::KrxError;
use jeed_krx::TrCode as T;
use jeed_krx::decode::dispatch::{decode, handles, message_len};
use jeed_wire::{WireKind, WireRecord};

fn kind_of(msg: &[u8]) -> WireKind {
    let mut out = WireRecord::zeroed();
    decode(msg, RECV_NS, &mut out).expect("decodes");
    assert_eq!(out.validate(), Ok(()));
    out.kind().expect("a known kind")
}

#[test]
fn every_decoder_this_build_has_is_reachable_from_the_trcode_alone() {
    // The receive loop reads five bytes and gets a record back. If a decoder is
    // not in this list it exists but can never run.
    assert_eq!(kind_of(&B6::kospi200(kospi200_book()).build()), WireKind::Quote);
    assert_eq!(kind_of(&G7::kospi200(kospi200_book()).build()), WireKind::TradeQuote);
    assert_eq!(kind_of(&A3::kospi200().build()), WireKind::Trade);
    assert_eq!(kind_of(&V1::kospi200().build()), WireKind::PriceLimit);
    assert_eq!(kind_of(&Q2::kospi200().build()), WireKind::DynamicPriceLimit);
    assert_eq!(kind_of(&StockB6::samsung().build()), WireKind::Quote);
    assert_eq!(kind_of(&EtfB7::kodex200().build()), WireKind::Quote);
    assert_eq!(kind_of(&EquityA3::samsung().build()), WireKind::Trade);
    assert_eq!(kind_of(&BondB6::ktb(ktb_book()).build()), WireKind::Quote);
    assert_eq!(kind_of(&BondA3::ktb().build()), WireKind::Trade);
    assert_eq!(kind_of(&BondG7::ktb(ktb_book()).build()), WireKind::TradeQuote);
    assert_eq!(kind_of(&M4::derivative_session_start().build()), WireKind::MarketSchedule);
}

#[test]
fn one_data_class_reaches_three_markets_and_not_one_byte_of_shared_layout() {
    // `B6` is 324 B in 파생, 590 B in 주식, 462 B in 채권. This is why the
    // dispatch is two levels and not a match on the first two bytes alone.
    assert_eq!(message_len(T::new(*b"B601F")), Some(324));
    assert_eq!(message_len(T::new(*b"B601S")), Some(590));
    assert_eq!(message_len(T::new(*b"B601K")), Some(462));
}

#[test]
fn the_derivative_depth_is_taken_from_the_product_group() {
    assert_eq!(message_len(T::new(*b"B605F")), Some(554), "주식옵션 ten-deep");
    assert_eq!(message_len(T::new(*b"B618F")), Some(554), "개별주식 위클리옵션");
    assert_eq!(message_len(T::new(*b"B604F")), Some(324), "주식선물 truncated to five");
    assert_eq!(message_len(T::new(*b"G705F")), Some(661));
    assert_eq!(message_len(T::new(*b"G704F")), Some(431));
}

#[test]
fn a_ten_deep_message_is_dispatched_to_the_ten_deep_decoder() {
    // The highest-consequence branch in the whole table: B604F is 40% of the
    // feed and reading it ten-deep loses every one of them.
    let mut deep = single_stock_book();
    deep.extend(single_stock_book());
    let mut msg = B6::kospi200(deep);
    msg.header.trcode = "B605F";
    msg.expected_price = "000000000";

    let mut out = WireRecord::zeroed();
    decode(&msg.build(), RECV_NS, &mut out).expect("decodes");
    assert_eq!(out.header.depth, 10);
}

#[test]
fn a_five_deep_message_behind_a_ten_deep_trcode_is_refused_by_length() {
    // Mis-dispatch does not parse garbage: the frame check runs first, so the
    // message is simply the wrong length for the interface.
    let mut msg = B6::kospi200(single_stock_book());
    msg.header.trcode = "B605F"; // ten-deep code on a five-deep message
    msg.expected_price = "000000000";

    let mut out = WireRecord::zeroed();
    assert_eq!(
        decode(&msg.build(), RECV_NS, &mut out),
        Err(KrxError::Length { expected: 554, actual: 324 })
    );
    assert_eq!(out, WireRecord::zeroed());
}

#[test]
fn an_unknown_trcode_is_reported_rather_than_ignored() {
    // A code this build does not decode and a corrupt datagram are different
    // problems — the first is a configuration question, the second is a bad
    // message — so the receive loop needs to be told which happened.
    let mut msg = B6::kospi200(kospi200_book()).build();
    msg[..5].copy_from_slice(b"ZZ99F");

    let mut out = WireRecord::zeroed();
    assert_eq!(
        decode(&msg, RECV_NS, &mut out),
        Err(KrxError::UnknownTrCode { code: T::new(*b"ZZ99F") })
    );
    assert_eq!(out, WireRecord::zeroed());
}

#[test]
fn markets_this_build_does_not_decode_are_not_claimed() {
    // 금현물, 배출권, 소액채권, REPO. Claiming one would mean applying some
    // other market's layout to its bytes.
    for code in [b"B601G", b"B601E", b"B601M", b"B601R", b"A301G", b"G701R"] {
        assert!(!handles(T::new(*code)), "{}", T::new(*code));
        assert_eq!(message_len(T::new(*code)), None);
    }
}

#[test]
fn the_length_check_agrees_with_what_the_decoder_actually_requires() {
    // The receive loop uses `message_len` to size a check before it commits a
    // ring slot, so a disagreement here would either drop good messages or let
    // bad ones through to the decoder.
    let cases: [(&[u8; 5], Vec<u8>); 12] = [
        (b"B601F", B6::kospi200(kospi200_book()).build()),
        (b"G701F", G7::kospi200(kospi200_book()).build()),
        (b"A301F", A3::kospi200().build()),
        (b"V101F", V1::kospi200().build()),
        (b"Q201F", Q2::kospi200().build()),
        (b"B601S", StockB6::samsung().build()),
        (b"B703S", EtfB7::kodex200().build()),
        (b"A301S", EquityA3::samsung().build()),
        (b"B601K", BondB6::ktb(ktb_book()).build()),
        (b"A301K", BondA3::ktb().build()),
        (b"G701K", BondG7::ktb(ktb_book()).build()),
        (b"M401F", M4::derivative_session_start().build()),
    ];
    for (code, msg) in cases {
        assert_eq!(
            message_len(T::new(*code)),
            Some(msg.len()),
            "{} length",
            T::new(*code)
        );
        assert!(handles(T::new(*code)));
    }
}

#[test]
fn the_schedule_message_is_claimed_for_every_market() {
    for code in [b"M401F", b"M401S", b"M401K", b"M401E"] {
        assert_eq!(message_len(T::new(*code)), Some(83), "{}", T::new(*code));
    }
}
