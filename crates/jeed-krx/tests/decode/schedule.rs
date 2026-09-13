//! `jeed_krx::decode::schedule` — `M4` 장운영스케줄공개.

use crate::common::{M4, RECV_NS};
use jeed_krx::KrxError;
use jeed_krx::decode::schedule::{DECODER, MESSAGE_LEN, handles};
use jeed_wire::{WireKind, WireRecord, expansion_direction, header_flags, schedule_flags};

fn decode(msg: &[u8]) -> WireRecord {
    let mut out = WireRecord::zeroed();
    DECODER.decode(msg, RECV_NS, &mut out).expect("decodes");
    out
}

#[test]
fn the_message_is_the_length_the_spec_defines() {
    assert_eq!(MESSAGE_LEN, 83); // IFMSRPD0019
}

#[test]
fn a_session_start_notice_round_trips() {
    let rec = decode(&M4::derivative_session_start().build());

    assert_eq!(rec.validate(), Ok(()));
    assert_eq!(rec.kind(), Ok(WireKind::MarketSchedule));

    let s = rec.market_schedule().unwrap();
    assert_eq!(s.board_id, *b"G1");
    assert_eq!(s.board_event_id, *b"BS1");
    assert_eq!(s.session_action, *b"BS");
    assert_eq!(s.session_id, *b"40");
    assert_eq!(s.market_operation_product_id, *b"101");
    assert_eq!(s.information_category, *b"01F");
    assert_eq!(s.event_time_of_day, 9 * 3600 * 1_000_000_000);
}

#[test]
fn a_product_wide_notice_is_not_scoped_to_an_instrument() {
    // The 종목코드 is twelve spaces. That is the one scope question the handler
    // can answer without a venue codebook, so it is the one that becomes a flag.
    let rec = decode(&M4::derivative_session_start().build());
    let s = rec.market_schedule().unwrap();

    assert_eq!(s.schedule_flags & schedule_flags::INSTRUMENT_SCOPED, 0);
    assert_eq!(s.product_id, *b"KR4101V9000");
}

#[test]
fn an_instrument_scoped_notice_says_so_and_carries_the_isin() {
    let rec = decode(&M4::expansion_notice().build());
    let s = rec.market_schedule().unwrap();

    assert!(s.schedule_flags & schedule_flags::INSTRUMENT_SCOPED != 0);
    assert_eq!(rec.header.isin, *b"KR4101V90009");
}

#[test]
fn an_expansion_notice_carries_its_direction_and_its_time() {
    let rec = decode(&M4::expansion_notice().build());
    let s = rec.market_schedule().unwrap();

    assert_eq!(s.expansion_direction, expansion_direction::UP);
    assert!(s.schedule_flags & schedule_flags::EXPECTED_TIME_VALID != 0);
    assert_eq!(s.expected_time_of_day, (10 * 3600 + 15 * 60) * 1_000_000_000);
    assert_eq!(s.step, 1);
}

#[test]
fn a_notice_that_announces_no_expansion_says_not_applicable() {
    // Every non-derivative market sends a space here, and so does a derivative
    // notice that is not about the band.
    let rec = decode(&M4::derivative_session_start().build());
    let s = rec.market_schedule().unwrap();

    assert_eq!(s.expansion_direction, expansion_direction::NOT_APPLICABLE);
    assert_eq!(s.schedule_flags & schedule_flags::EXPECTED_TIME_VALID, 0);
}

#[test]
fn the_event_time_is_not_stamped_as_a_venue_timestamp() {
    // 보드이벤트시작시각 is when the event starts, which can be in the future —
    // it is a schedule. Stamping it as venue_ns would let a consumer age the
    // market against a time that has not happened.
    let rec = decode(&M4::expansion_notice().build());
    assert!(!rec.header.has(header_flags::VENUE_TIME_VALID));
    assert_eq!(rec.header.recv_ns, RECV_NS);
}

#[test]
fn a_downward_expansion_is_a_distinct_direction() {
    let mut msg = M4::expansion_notice();
    msg.expansion = b'2';
    assert_eq!(
        decode(&msg.build()).market_schedule().unwrap().expansion_direction,
        expansion_direction::DOWN
    );
}

#[test]
fn a_message_of_the_wrong_length_is_refused() {
    let mut msg = M4::derivative_session_start().build();
    msg.truncate(80);
    let mut out = WireRecord::zeroed();
    assert_eq!(
        DECODER.decode(&msg, RECV_NS, &mut out),
        Err(KrxError::Length { expected: 83, actual: 80 })
    );
}

#[test]
fn every_market_sends_this_one_interface() {
    // The reason this module sits beside the market modules rather than inside
    // one: 파생, 증권, 채권, 금현물, 배출권 all send the same 83 bytes.
    use jeed_krx::TrCode as T;
    for code in [b"M401F", b"M401S", b"M401Q", b"M401B", b"M401K", b"M401R", b"M401E"] {
        assert!(handles(T::new(*code)), "{}", T::new(*code));
    }
    assert!(!handles(T::new(*b"A301F")));
}
