//! `jeed_krx::decode::derivative::trade` — `A3` 파생 체결.

mod common;

use common::{A3, G7, RECV_NS, VENUE_NS, kospi200_book};
use jeed_krx::KrxError;
use jeed_krx::decode::derivative::trade::{DECODER, MESSAGE_LEN, handles};
use jeed_wire::{Scale, WireKind, WireRecord, header_flags, trade_flags, trade_kind};

fn decode(msg: &[u8]) -> WireRecord {
    let mut out = WireRecord::zeroed();
    DECODER.decode(msg, RECV_NS, &mut out).expect("decodes");
    out
}

#[test]
fn the_message_is_the_length_the_spec_defines() {
    assert_eq!(MESSAGE_LEN, 173); // IFMSRPD0036
    assert_eq!(DECODER.message_len(), 173);
}

#[test]
fn a_print_round_trips_into_a_trade_record() {
    let rec = decode(&A3::kospi200().build());

    assert_eq!(rec.validate(), Ok(()));
    assert_eq!(rec.kind(), Ok(WireKind::Trade));
    assert_eq!(rec.header.isin, *b"KR4101V90009");

    let t = rec.trade().unwrap();
    assert_eq!(t.price, 93700);
    assert_eq!(t.qty, 3);
    assert_eq!(rec.header.price_scale(), Ok(Scale::S2));
}

#[test]
fn a3_and_g7_read_the_same_bytes_to_the_same_trade() {
    // The whole reason `fill_trade` is shared: A3 is G7 with the book cut off,
    // so if these two ever disagree one of them has drifted.
    let a3_rec = decode(&A3::kospi200().build());
    let from_a3 = *a3_rec.trade().unwrap();

    let mut g7_out = WireRecord::zeroed();
    jeed_krx::decode::derivative::trade_quote::FIVE_DEEP
        .decode(&G7::kospi200(kospi200_book()).build(), RECV_NS, &mut g7_out)
        .expect("decodes");
    let from_g7 = g7_out.trade_quote().unwrap().trade;

    assert_eq!(from_a3, from_g7);
}

#[test]
fn the_dynamic_band_rides_along_with_the_print() {
    let rec = decode(&A3::kospi200().build());
    let t = rec.trade().unwrap();
    assert_eq!(t.dyn_limits(), Some((94635, 92765)));
    assert!(t.trade_flags & trade_flags::DYN_LIMIT_VALID != 0);
}

#[test]
fn an_instrument_outside_the_regime_is_not_given_a_zero_band() {
    let mut msg = A3::kospi200();
    msg.dyn_limits = ("000000.00", "000000.00");
    assert_eq!(decode(&msg.build()).trade().unwrap().dyn_limits(), None);
}

#[test]
fn the_cumulative_quantity_is_carried_and_marked() {
    assert_eq!(decode(&A3::kospi200().build()).trade().unwrap().cumulative_qty(), Some(166_478));
}

#[test]
fn the_aggressor_code_maps_to_the_wire_encoding() {
    for (code, expected) in
        [(b'1', trade_kind::SELL), (b'2', trade_kind::BUY), (b' ', trade_kind::UNKNOWN)]
    {
        let mut msg = A3::kospi200();
        msg.aggressor = code;
        assert_eq!(decode(&msg.build()).trade().unwrap().trade_kind, expected);
    }
}

#[test]
fn a_record_with_no_book_is_not_a_record_with_an_empty_book() {
    // A3 carries no quote at all. Setting BID_EMPTY/ASK_EMPTY here would tell
    // the consumer both sides are void, which is a statement about the market
    // this message never made.
    let rec = decode(&A3::kospi200().build());
    assert_eq!(rec.header.depth, 0);
    assert!(!rec.header.has(header_flags::BID_EMPTY));
    assert!(!rec.header.has(header_flags::ASK_EMPTY));
}

#[test]
fn the_exchange_timestamp_is_assembled_and_marked_valid() {
    let rec = decode(&A3::kospi200().build());
    assert!(rec.header.has(header_flags::VENUE_TIME_VALID));
    assert_eq!(rec.header.venue_ns, VENUE_NS);
}

#[test]
fn a_spread_print_may_be_negative() {
    let mut msg = A3::kospi200();
    msg.price = "-00000.85";
    assert_eq!(decode(&msg.build()).trade().unwrap().price, -85);
}

#[test]
fn a_corrupt_field_leaves_the_output_untouched() {
    let mut msg = A3::kospi200().build();
    msg[47] = b'X'; // sign byte of 체결가격

    let mut out = WireRecord::zeroed();
    assert!(DECODER.decode(&msg, RECV_NS, &mut out).is_err());
    assert_eq!(out, WireRecord::zeroed(), "no partial update");
}

#[test]
fn a_message_of_the_wrong_length_is_refused_before_any_field_is_read() {
    let mut msg = A3::kospi200().build();
    msg.truncate(170);

    let mut out = WireRecord::zeroed();
    assert_eq!(
        DECODER.decode(&msg, RECV_NS, &mut out),
        Err(KrxError::Length { expected: 173, actual: 170 })
    );
}

#[test]
fn every_derivative_product_group_sends_the_same_shape() {
    // Unlike B6/G7 there is nothing to choose here: deep book or not, 체결 is
    // 173 bytes.
    use jeed_krx::TrCode as T;
    for code in [b"A301F", b"A304F", b"A305F", b"A310F", b"A318F"] {
        assert!(handles(T::new(*code)), "{}", T::new(*code));
    }
    assert!(!handles(T::new(*b"A301S")), "증권 체결 is a different interface");
    assert!(!handles(T::new(*b"B601F")));
}
