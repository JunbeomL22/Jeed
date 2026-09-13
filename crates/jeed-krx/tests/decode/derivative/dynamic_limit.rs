//! `jeed_krx::decode::derivative::dynamic_limit` — `Q2` 파생 동적상하한가
//! 적용 및 해제.

use crate::common::{Q2, RECV_NS, V1, VENUE_NS};
use jeed_krx::decode::derivative::{dynamic_limit, price_limit};
use jeed_wire::{Scale, WireKind, WireRecord, dyn_limit_action};

fn decode(msg: &[u8]) -> WireRecord {
    let mut out = WireRecord::zeroed();
    dynamic_limit::DECODER.decode(msg, RECV_NS, &mut out).expect("decodes");
    out
}

#[test]
fn the_message_is_the_length_the_spec_defines() {
    assert_eq!(dynamic_limit::MESSAGE_LEN, 65); // IFMSRPD0042
}

#[test]
fn an_applied_dynamic_band_round_trips() {
    let rec = decode(&Q2::kospi200().build());

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
    let rec = decode(&msg.build());
    let d = rec.dynamic_price_limit().unwrap();

    assert_eq!(d.action, dyn_limit_action::RELEASED);
    assert_eq!(d.band(), None, "the prices are still in the message and still not a band");
}

#[test]
fn an_unrecognised_action_reads_as_no_band_rather_than_a_guess() {
    let mut msg = Q2::kospi200();
    msg.action = b'9';
    let rec = decode(&msg.build());
    let d = rec.dynamic_price_limit().unwrap();

    assert_eq!(d.action, dyn_limit_action::UNKNOWN);
    assert_eq!(d.band(), None);
    assert_eq!(rec.validate(), Ok(()), "unknown is a value the wire allows, not a corrupt record");
}

#[test]
fn the_processing_time_is_microseconds_unlike_v1() {
    let rec = decode(&Q2::kospi200().build());
    assert_eq!(rec.header.venue_ns, VENUE_NS);
    assert_eq!(
        rec.dynamic_price_limit().unwrap().applied_time_of_day,
        (9 * 3600 + 60) * 1_000_000_000 + 123_456_000
    );
}

#[test]
fn the_two_bands_are_different_records_and_stay_that_way() {
    let daily = super::price_limit::decode(&V1::kospi200().build());
    let intraday = decode(&Q2::kospi200().build());

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
