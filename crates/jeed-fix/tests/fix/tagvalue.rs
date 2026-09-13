//! Tests for `src/data/fix/tagvalue.rs` — field scanning and value parsers.

use super::soh;
use jeed_fix::tagvalue::{days_from_civil, Field};
use jeed_fix::{
    parse_scaled_decimal, parse_u64, parse_utc_date, parse_utc_time, parse_utc_timestamp,
    split_field, DecimalError, Fields, FixError, MsgType,
};
use jeed_wire::Scale;

fn field<'a>(tag: u32, value: &'a str) -> Field<'a> {
    Field { tag, value: value.as_bytes() }
}

#[test]
fn parse_u64_accepts_only_plain_digits() {
    assert_eq!(parse_u64(b"0"), Some(0));
    assert_eq!(parse_u64(b"1450"), Some(1450));
    assert_eq!(parse_u64(b"9999999999999999999"), Some(9_999_999_999_999_999_999));
    assert_eq!(parse_u64(b""), None);
    assert_eq!(parse_u64(b" 12"), None);
    assert_eq!(parse_u64(b"12 "), None);
    assert_eq!(parse_u64(b"-1"), None);
    assert_eq!(parse_u64(b"1.0"), None);
    assert_eq!(parse_u64(b"12345678901234567890"), None, "20 digits would overflow");
}

#[test]
fn scaled_decimal_is_exact_on_the_target_scale() {
    assert_eq!(parse_scaled_decimal(b"1450.00", Scale::S2), Ok(145_000));
    assert_eq!(parse_scaled_decimal(b"1450.9", Scale::S2), Ok(145_090));
    assert_eq!(parse_scaled_decimal(b"1450", Scale::S2), Ok(145_000));
    assert_eq!(parse_scaled_decimal(b"1450.", Scale::S2), Ok(145_000));
    assert_eq!(parse_scaled_decimal(b".5", Scale::S2), Ok(50));
    assert_eq!(parse_scaled_decimal(b"-0.01", Scale::S2), Ok(-1));
    assert_eq!(parse_scaled_decimal(b"5000000", Scale::S0), Ok(5_000_000));
}

#[test]
fn scaled_decimal_refuses_to_round_away_a_digit() {
    // A third decimal on an S2 price is a configuration error, not a price.
    assert_eq!(parse_scaled_decimal(b"1450.005", Scale::S2), Err(DecimalError::Precision));
    // Zeros past the scale carry no information, so they are accepted.
    assert_eq!(parse_scaled_decimal(b"1450.1000", Scale::S2), Ok(145_010));
    assert_eq!(parse_scaled_decimal(b"", Scale::S2), Err(DecimalError::Malformed));
    assert_eq!(parse_scaled_decimal(b"-", Scale::S2), Err(DecimalError::Malformed));
    assert_eq!(parse_scaled_decimal(b"+1", Scale::S2), Err(DecimalError::Malformed));
    assert_eq!(parse_scaled_decimal(b"1,450", Scale::S2), Err(DecimalError::Malformed));
    assert_eq!(parse_scaled_decimal(b"1e3", Scale::S2), Err(DecimalError::Malformed));
}

#[test]
fn field_accessors_name_the_offending_tag() {
    assert_eq!(field(271, "5000000").as_u64(), Ok(5_000_000));
    assert_eq!(field(268, "2").as_u32(), Ok(2));
    assert_eq!(field(269, "0").first_byte(), Ok(b'0'));
    assert_eq!(field(269, "01").first_byte(), Err(FixError::BadValue { tag: 269 }));
    assert_eq!(field(270, "1450.00").as_scaled(Scale::S2), Ok(145_000));
    assert_eq!(
        field(270, "1450.005").as_scaled(Scale::S2),
        Err(FixError::Precision { tag: 270 }),
        "a lost digit is reported as precision, not as a bad value"
    );
    assert_eq!(field(270, "x").as_scaled(Scale::S2), Err(FixError::BadValue { tag: 270 }));
    assert_eq!(
        field(271, "-1").as_scaled_unsigned(Scale::S0),
        Err(FixError::BadValue { tag: 271 })
    );
    assert!(field(43, "Y").is_yes());
    assert!(!field(43, "N").is_yes());
}

#[test]
fn epoch_and_capture_dates_convert() {
    assert_eq!(days_from_civil(1970, 1, 1), 0);
    assert_eq!(days_from_civil(1969, 12, 31), -1);
    assert_eq!(days_from_civil(2000, 3, 1), 11_017);
    // 2026-02-02 00:00:00 UTC, the first day of the SMBS capture.
    assert_eq!(parse_utc_date(b"20260202"), Some(super::DAY_NS));
    // 2026 is not a leap year; 2024 is.
    assert_eq!(
        parse_utc_date(b"20240301").unwrap() - parse_utc_date(b"20240228").unwrap(),
        2 * 86_400 * 1_000_000_000
    );
    assert_eq!(parse_utc_date(b"2026020"), None);
    assert_eq!(parse_utc_date(b"20261301"), None);
    assert_eq!(parse_utc_date(b"20260200"), None);
}

#[test]
fn utc_time_takes_fractions_up_to_nanoseconds() {
    assert_eq!(parse_utc_time(b"00:00:00"), Some(0));
    assert_eq!(parse_utc_time(b"00:00:00.005"), Some(5_000_000));
    assert_eq!(parse_utc_time(b"00:00:00.000005"), Some(5_000));
    assert_eq!(parse_utc_time(b"00:00:00.000000005"), Some(5));
    assert_eq!(parse_utc_time(b"23:59:59.999"), Some(86_399_999_000_000));
    assert_eq!(parse_utc_time(b"23:59:60"), Some(86_400_000_000_000), "leap second");
    assert_eq!(parse_utc_time(b"24:00:00"), None);
    assert_eq!(parse_utc_time(b"00:60:00"), None);
    assert_eq!(parse_utc_time(b"000000"), None);
    assert_eq!(parse_utc_time(b"00:00:00,5"), None);
    assert_eq!(parse_utc_time(b"00:00:00.0000000005"), None, "10 fractional digits");
}

#[test]
fn utc_timestamp_joins_date_and_time() {
    assert_eq!(
        parse_utc_timestamp(b"20260202-00:00:00.005"),
        Some(super::DAY_NS + 5_000_000)
    );
    assert_eq!(parse_utc_timestamp(b"20260202T00:00:00"), None, "separator must be '-'");
    assert_eq!(parse_utc_timestamp(b"20260202-00:00"), None);
}

#[test]
fn split_field_consumes_exactly_one_field() {
    let bytes = soh("35=W|49=SMBS|");
    let (f, used) = split_field(&bytes).unwrap();
    assert_eq!((f.tag, f.value), (35, b"W".as_slice()));
    assert_eq!(used, 5);
    let (f, used) = split_field(&bytes[used..]).unwrap();
    assert_eq!((f.tag, f.value), (49, b"SMBS".as_slice()));
    assert_eq!(used, 8);

    assert!(split_field(&soh("35=W")).is_none(), "no SOH ⇒ incomplete");
    assert!(split_field(&soh("=W|")).is_none(), "empty tag");
    assert!(split_field(&soh("3x=W|")).is_none(), "non-numeric tag");
    assert!(split_field(&soh("35W|")).is_none(), "no '='");
}

#[test]
fn fields_iterates_a_body_and_reports_one_error_then_stops() {
    let body = soh("269=0|270=1450.00|271=5000000|");
    let got: Vec<_> = Fields::new(&body).map(|f| f.unwrap().tag).collect();
    assert_eq!(got, vec![269, 270, 271]);

    // An empty value is legal FIX-wise as far as the scanner is concerned.
    let body = soh("58=|35=W|");
    let got: Vec<_> = Fields::new(&body).map(|f| f.unwrap()).collect();
    assert!(got[0].value.is_empty());

    // A truncated tail is one error, then the iterator ends.
    let body = soh("269=0|270=1450.00");
    let mut it = Fields::new(&body);
    assert_eq!(it.next().unwrap().unwrap().tag, 269);
    assert_eq!(it.next().unwrap(), Err(FixError::Field { offset: 6 }));
    assert!(it.next().is_none());
}

#[test]
fn msg_type_classifies_admin_and_market_data() {
    assert_eq!(MsgType::from_value(b"W"), MsgType::MarketDataSnapshot);
    assert_eq!(MsgType::from_value(b"X"), MsgType::MarketDataIncremental);
    assert!(MsgType::from_value(b"W").is_market_data());
    assert!(MsgType::from_value(b"X").is_market_data());
    assert!(!MsgType::from_value(b"W").is_admin());

    for (v, expected) in [
        (&b"0"[..], MsgType::Heartbeat),
        (b"1", MsgType::TestRequest),
        (b"2", MsgType::ResendRequest),
        (b"3", MsgType::Reject),
        (b"4", MsgType::SequenceReset),
        (b"5", MsgType::Logout),
        (b"A", MsgType::Logon),
    ] {
        assert_eq!(MsgType::from_value(v), expected);
        assert!(expected.is_admin());
        assert!(!expected.is_market_data());
    }

    // An execution report is neither: the order session (quickfix-rs) owns it.
    assert_eq!(MsgType::from_value(b"8"), MsgType::Other { first: b'8' });
    assert!(!MsgType::from_value(b"8").is_admin());
    assert_eq!(MsgType::from_value(b""), MsgType::Other { first: 0 });
}
