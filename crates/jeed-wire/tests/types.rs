//! `jeed_wire::types` — the value types carried on the wire.

use jeed_wire::{ISIN_LEN, Scale, Venue};

#[test]
fn venue_bytes_are_stable() {
    // These discriminants are ABI. Changing one is a wire format change.
    assert_eq!(Venue::Krx.as_u8(), 0);
    assert_eq!(Venue::Nxt.as_u8(), 1);
    assert_eq!(Venue::Smbs.as_u8(), 2);
}

#[test]
fn venue_round_trips_through_its_byte() {
    for v in [Venue::Krx, Venue::Nxt, Venue::Smbs] {
        assert_eq!(Venue::from_u8(v.as_u8()), Some(v));
        assert_eq!(Venue::from_tag(v.as_str()), Some(v));
    }
}

#[test]
fn unknown_venue_byte_is_rejected() {
    assert_eq!(Venue::from_u8(3), None);
    assert_eq!(Venue::from_u8(u8::MAX), None);
    assert_eq!(Venue::from_tag("krx"), None, "tag matching is case-sensitive");
}

#[test]
fn scale_byte_is_its_decimal_count() {
    for d in 0..=8u8 {
        let s = Scale::from_decimals(d as usize).expect("0..=8 are valid scales");
        assert_eq!(s.decimals(), d);
        assert_eq!(s as u8, d);
    }
    assert_eq!(Scale::from_decimals(9), None);
}

#[test]
fn scale_divisor_matches_decimals() {
    assert_eq!(Scale::S0.divisor(), 1);
    assert_eq!(Scale::S2.divisor(), 100);
    assert_eq!(Scale::S8.divisor(), 100_000_000);
}

#[test]
fn scaled_integer_keeps_exact_value() {
    // 1450.10 at S2 is 145_010 — the point of scaling is that this is exact.
    let raw: i64 = 145_010;
    assert_eq!(raw / Scale::S2.divisor() as i64, 1450);
    assert_eq!(raw % Scale::S2.divisor() as i64, 10);
}

#[test]
fn isin_is_twelve_bytes() {
    assert_eq!(ISIN_LEN, 12);
    let isin: jeed_wire::Isin = *b"KR4A01690002";
    assert_eq!(isin.len(), ISIN_LEN);
}
