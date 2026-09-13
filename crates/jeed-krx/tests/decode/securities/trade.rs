//! `jeed_krx::decode::securities::trade` — `A3` 증권 체결.
//!
//! One interface for 주식 and the LP products alike, which is why
//! `stock::trade` and `etf::trade` point at it instead of re-spelling the
//! offsets.

use crate::common::{EquityA3, RECV_NS, VENUE_NS};
use jeed_krx::KrxError;
use jeed_krx::decode::securities::trade::{DECODER, MESSAGE_LEN, handles};
use jeed_krx::decode::{etf, stock};
use jeed_wire::{Scale, WireKind, WireRecord, header_flags, trade_flags, trade_kind};

fn decode(msg: &[u8]) -> WireRecord {
    let mut out = WireRecord::zeroed();
    DECODER.decode(msg, RECV_NS, &mut out).expect("decodes");
    out
}

#[test]
fn the_message_is_the_length_the_spec_defines() {
    assert_eq!(MESSAGE_LEN, 186); // IFMSRPD0004
}

#[test]
fn a_print_round_trips_into_a_trade_record() {
    let rec = decode(&EquityA3::samsung().build());

    assert_eq!(rec.validate(), Ok(()));
    assert_eq!(rec.kind(), Ok(WireKind::Trade));
    assert_eq!(rec.header.symbol_bytes(), b"KR7005930003");
    assert_eq!(rec.header.price_scale(), Ok(Scale::S0));

    let t = rec.trade().unwrap();
    assert_eq!(t.price, 74100);
    assert_eq!(t.qty, 37);
    assert_eq!(t.cumulative_qty(), Some(4_812_355));
}

#[test]
fn the_aggressor_code_maps_to_the_wire_encoding() {
    for (code, expected) in
        [(b'1', trade_kind::SELL), (b'2', trade_kind::BUY), (b'0', trade_kind::UNKNOWN)]
    {
        let mut msg = EquityA3::samsung();
        msg.aggressor = code;
        assert_eq!(decode(&msg.build()).trade().unwrap().trade_kind, expected);
    }
}

#[test]
fn an_auction_cross_has_no_aggressor_but_the_field_was_still_there() {
    let mut msg = EquityA3::samsung();
    msg.aggressor = b' ';
    let rec = decode(&msg.build());
    let t = rec.trade().unwrap();

    assert_eq!(t.trade_kind, trade_kind::UNKNOWN);
    assert_ne!(t.trade_kind, trade_kind::NONE, "the channel does have the field");
}

#[test]
fn there_is_no_dynamic_band_on_an_equity_print() {
    // 실시간가격제한 is a derivatives regime. Leaving the flag clear is what
    // says "this market has no such thing" rather than "the band is zero".
    let rec = decode(&EquityA3::samsung().build());
    let t = rec.trade().unwrap();
    assert_eq!(t.dyn_limits(), None);
    assert_eq!(t.trade_flags & trade_flags::DYN_LIMIT_VALID, 0);
}

#[test]
fn the_top_of_book_in_the_tail_is_deliberately_dropped() {
    // The message ends with 매도/매수최우선호가가격 — prices with no sizes.
    // Writing them as levels with qty 0 would say nothing rests there, which is
    // a claim about the market the message never made.
    let mut msg = EquityA3::samsung();
    msg.bbo = ("00000099999", "00000011111");
    let rec = decode(&msg.build());

    assert_eq!(rec.kind(), Ok(WireKind::Trade), "a trade, not a trade+quote");
    assert_eq!(rec.header.depth, 0);
    assert!(!rec.header.has(header_flags::BID_EMPTY));
    assert!(!rec.header.has(header_flags::ASK_EMPTY));
}

#[test]
fn the_exchange_timestamp_is_assembled_and_marked_valid() {
    let rec = decode(&EquityA3::samsung().build());
    assert!(rec.header.has(header_flags::VENUE_TIME_VALID));
    assert_eq!(rec.header.venue_ns, VENUE_NS);
}

#[test]
fn a_corrupt_field_leaves_the_output_untouched() {
    let mut msg = EquityA3::samsung().build();
    msg[59] = b'X'; // sign byte of 체결가격

    let mut out = WireRecord::zeroed();
    assert!(DECODER.decode(&msg, RECV_NS, &mut out).is_err());
    assert_eq!(out, WireRecord::zeroed(), "no partial update");
}

#[test]
fn a_message_of_the_wrong_length_is_refused() {
    let mut msg = EquityA3::samsung().build();
    msg.truncate(180);
    let mut out = WireRecord::zeroed();
    assert_eq!(
        DECODER.decode(&msg, RECV_NS, &mut out),
        Err(KrxError::Length { expected: 186, actual: 180 })
    );
}

#[test]
fn both_markets_point_at_this_one_decoder() {
    use jeed_krx::TrCode as T;
    // Same decoder, reached by two names — so there is no second copy of these
    // offsets to drift.
    assert_eq!(stock::trade::MESSAGE_LEN, MESSAGE_LEN);
    assert_eq!(etf::trade::MESSAGE_LEN, MESSAGE_LEN);

    assert!(stock::trade::handles(T::new(*b"A301S")));
    assert!(!stock::trade::handles(T::new(*b"A303S")), "ETF is the other module's");
    assert!(etf::trade::handles(T::new(*b"A303S")));
    assert!(!etf::trade::handles(T::new(*b"A301S")));

    // The shared one claims the whole market, which is what dispatch uses.
    for code in [b"A301S", b"A301Q", b"A301X", b"A302S", b"A303S", b"A304S", b"A305S"] {
        assert!(handles(T::new(*code)), "{}", T::new(*code));
    }
    assert!(!handles(T::new(*b"A301F")), "파생");
    assert!(!handles(T::new(*b"A301K")), "채권");
}
