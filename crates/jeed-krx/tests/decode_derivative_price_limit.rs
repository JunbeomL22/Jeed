//! `jeed_krx::decode::derivative::{price_limit, dynamic_limit}` — the two
//! price-limit channels, `V1` and `Q2`.
//!
//! They are tested together because the thing worth protecting is that they
//! stay **apart**: `V1` is the daily band that widens in stages, `Q2` is the
//! intraday band that moves with every print, and an order has to clear both
//! (`documents/krx/실시간가격제한.md`).

mod common;

use common::{Q2, RECV_NS, V1, VENUE_NS};
use jeed_krx::KrxError;
use jeed_krx::decode::derivative::dynamic_limit;
use jeed_krx::decode::derivative::price_limit;
use jeed_wire::{Scale, WireKind, WireRecord, dyn_limit_action, header_flags};

fn decode_v1(msg: &[u8]) -> WireRecord {
    let mut out = WireRecord::zeroed();
    price_limit::DECODER.decode(msg, RECV_NS, &mut out).expect("decodes");
    out
}

fn decode_q2(msg: &[u8]) -> WireRecord {
    let mut out = WireRecord::zeroed();
    dynamic_limit::DECODER.decode(msg, RECV_NS, &mut out).expect("decodes");
    out
}

#[test]
fn both_messages_are_the_length_the_spec_defines() {
    assert_eq!(price_limit::MESSAGE_LEN, 65); // IFMSRPD0043
    assert_eq!(dynamic_limit::MESSAGE_LEN, 65); // IFMSRPD0042
}

// ---------------------------------------------------------------------------
// V1 — the daily band
// ---------------------------------------------------------------------------

#[test]
fn a_widened_daily_band_round_trips() {
    let rec = decode_v1(&V1::kospi200().build());

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
    let rec = decode_v1(&msg.build());
    let p = rec.price_limit().unwrap();
    assert_eq!(p.upper_stage, 2);
    assert_eq!(p.lower_stage, 0);
}

#[test]
fn the_widening_time_is_milliseconds_not_microseconds() {
    // V1 is the odd one: 가격확대시각 is nine bytes (HHMMSSmmm) where every
    // other real-time clock field is twelve.
    let rec = decode_v1(&V1::kospi200().build());
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
    let rec = decode_v1(&msg.build());
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

// ---------------------------------------------------------------------------
// Q2 — the intraday band
// ---------------------------------------------------------------------------

#[test]
fn an_applied_dynamic_band_round_trips() {
    let rec = decode_q2(&Q2::kospi200().build());

    assert_eq!(rec.validate(), Ok(()));
    assert_eq!(rec.kind(), Ok(WireKind::DynamicPriceLimit));
    assert_eq!(rec.header.price_scale(), Ok(Scale::S2));

    let d = rec.dynamic_price_limit().unwrap();
    assert_eq!(d.action, dyn_limit_action::APPLIED);
    assert_eq!(d.band(), Some((94635, 92765)));
}

#[test]
fn a_release_is_the_thing_no_trade_can_tell_you() {
    // This is why Q2 is decoded at all. The band carried on G7/A3 says what it
    // is after a print; a release comes with no print, so a consumer watching
    // only the tape would go on fencing orders against a limit the exchange
    // has already taken down.
    let mut msg = Q2::kospi200();
    msg.action = b'2';
    let rec = decode_q2(&msg.build());
    let d = rec.dynamic_price_limit().unwrap();

    assert_eq!(d.action, dyn_limit_action::RELEASED);
    assert_eq!(d.band(), None, "the prices are still in the message and still not a band");
}

#[test]
fn an_unrecognised_action_reads_as_no_band_rather_than_a_guess() {
    let mut msg = Q2::kospi200();
    msg.action = b'9';
    let rec = decode_q2(&msg.build());
    let d = rec.dynamic_price_limit().unwrap();

    assert_eq!(d.action, dyn_limit_action::UNKNOWN);
    assert_eq!(d.band(), None);
    assert_eq!(rec.validate(), Ok(()), "unknown is a value the wire allows, not a corrupt record");
}

#[test]
fn the_processing_time_is_microseconds_unlike_v1() {
    let rec = decode_q2(&Q2::kospi200().build());
    assert_eq!(rec.header.venue_ns, VENUE_NS);
    assert_eq!(
        rec.dynamic_price_limit().unwrap().applied_time_of_day,
        (9 * 3600 + 60) * 1_000_000_000 + 123_456_000
    );
}

#[test]
fn the_two_bands_are_different_records_and_stay_that_way() {
    let daily = decode_v1(&V1::kospi200().build());
    let intraday = decode_q2(&Q2::kospi200().build());

    assert_ne!(daily.kind(), intraday.kind());
    // The intraday band sits inside the daily one; an order must clear both.
    let outer = daily.price_limit().unwrap();
    let (upper, lower) = intraday.dynamic_price_limit().unwrap().band().unwrap();
    assert!(upper < outer.upper_price);
    assert!(lower > outer.lower_price);
}

#[test]
fn each_decoder_claims_only_its_own_trcode() {
    use jeed_krx::TrCode as T;
    assert!(price_limit::handles(T::new(*b"V101F")));
    assert!(!price_limit::handles(T::new(*b"Q201F")));
    assert!(dynamic_limit::handles(T::new(*b"Q201F")));
    assert!(!dynamic_limit::handles(T::new(*b"V101F")));
    assert!(!price_limit::handles(T::new(*b"V101S")), "not a derivative channel");
}
