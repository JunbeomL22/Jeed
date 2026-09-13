//! `jeed_krx::decode::derivative::price_limit` — `V1` 파생 가격제한폭확대발동.
//!
//! The **daily** band, which widens in discrete stages. Its counterpart is
//! `Q2`, the intraday band that moves with every print; an order has to clear
//! both (`documents/krx/실시간가격제한.md`), so the pair is also tested
//! together in [`super::dynamic_limit`].

use crate::common::{RECV_NS, V1};
use jeed_krx::KrxError;
use jeed_krx::decode::derivative::price_limit;
use jeed_wire::{Scale, WireKind, WireRecord, header_flags};

pub(crate) fn decode(msg: &[u8]) -> WireRecord {
    let mut out = WireRecord::zeroed();
    price_limit::DECODER.decode(msg, RECV_NS, &mut out).expect("decodes");
    out
}

#[test]
fn the_message_is_the_length_the_spec_defines() {
    assert_eq!(price_limit::MESSAGE_LEN, 65); // IFMSRPD0043
}

#[test]
fn a_widened_daily_band_round_trips() {
    let rec = decode(&V1::kospi200().build());

    assert_eq!(rec.validate(), Ok(()));
    assert_eq!(rec.kind(), Ok(WireKind::PriceLimit));
    assert_eq!(rec.header.isin, *b"KR4101V90009");
    assert_eq!(rec.header.price_scale(), Ok(Scale::S2));

    let p = rec.price_limit().unwrap();
    assert_eq!(p.upper_price, 103_070);
    assert_eq!(p.lower_price, 84_330);
    assert!(p.upper_price > p.lower_price);
    assert_eq!(p.information_category, *b"01F");
    assert_eq!(p.board_id, *b"G1");
}

#[test]
fn the_two_stages_move_independently() {
    // The band can widen upward without widening downward, which is why these
    // are two fields and not one level.
    let mut msg = V1::kospi200();
    msg.stages = (2, 0);
    let rec = decode(&msg.build());
    let p = rec.price_limit().unwrap();
    assert_eq!(p.upper_stage, 2);
    assert_eq!(p.lower_stage, 0);
}

#[test]
fn the_widening_time_is_milliseconds_not_microseconds() {
    // V1 is the odd one: 가격확대시각 is nine bytes (HHMMSSmmm) where every
    // other real-time clock field is twelve.
    let rec = decode(&V1::kospi200().build());
    let p = rec.price_limit().unwrap();
    assert_eq!(p.applied_time_of_day, (9 * 3600 + 60) * 1_000_000_000 + 123_000_000);
    assert!(rec.header.has(header_flags::VENUE_TIME_VALID));
}

#[test]
fn a_blank_sequence_does_not_stop_the_decode() {
    // V103F still sends spaces here. Reading them as a number would make every
    // message look like a gap; ordering authority is producer_seq (CLAUDE.md).
    let mut msg = V1::kospi200();
    msg.header.sequence = None;
    let rec = decode(&msg.build());
    let p = rec.price_limit().unwrap();
    assert_eq!(p.sequence, 0, "blank is carried as zero, and documented as such");
}

#[test]
fn a_corrupt_limit_leaves_the_output_untouched() {
    let mut msg = V1::kospi200().build();
    msg[46] = b'X'; // sign byte of 상한가

    let mut out = WireRecord::zeroed();
    assert!(price_limit::DECODER.decode(&msg, RECV_NS, &mut out).is_err());
    assert_eq!(out, WireRecord::zeroed(), "no partial update");
}

#[test]
fn a_message_of_the_wrong_length_is_refused() {
    let mut msg = V1::kospi200().build();
    msg.truncate(60);
    let mut out = WireRecord::zeroed();
    assert_eq!(
        price_limit::DECODER.decode(&msg, RECV_NS, &mut out),
        Err(KrxError::Length { expected: 65, actual: 60 })
    );
}
