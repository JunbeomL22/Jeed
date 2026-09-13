//! `jeed_krx::field` — fixed-width ASCII readers.

use jeed_krx::field::{self, Decimal};
use jeed_krx::KrxError;
use jeed_wire::Scale;

fn dec(s: &str) -> Option<Decimal> {
    field::decimal(s.as_bytes(), 0).expect("parses")
}

#[test]
fn a_derivative_price_carries_its_own_scale() {
    // Nine bytes either way; the point's position is the instrument's, not the
    // message's, so it is read rather than looked up.
    assert_eq!(dec("000937.05"), Some(Decimal { value: 93705, decimals: 2 }));
    assert_eq!(dec("000012345"), Some(Decimal { value: 12345, decimals: 0 }));
    assert_eq!(dec("03456.789"), Some(Decimal { value: 3456789, decimals: 3 }));
}

#[test]
fn the_scale_is_the_one_the_wire_header_will_carry() {
    assert_eq!(dec("000937.05").unwrap().scale(), Some(Scale::S2));
    assert_eq!(dec("000012345").unwrap().scale(), Some(Scale::S0));
    assert_eq!(dec("03456.789").unwrap().scale(), Some(Scale::S3));
}

#[test]
fn a_blank_field_is_absent_not_zero() {
    // 정보분배일련번호 arrives blank on whole channels. Reading it as zero makes
    // every message look like a sequence gap (CLAUDE.md).
    assert_eq!(dec("         "), None);
    assert_eq!(field::uint(b"        ", 0), Ok(None));
    assert!(field::is_blank(b"   "));
    assert!(!field::is_blank(b"  0"));
}

#[test]
fn a_zero_field_is_present_and_zero() {
    // The converse of the above, and the reason the dynamic-limit conclusion
    // has to ride in a flag: KRX writes `000000.00`, not blanks, for
    // instruments the regime does not cover.
    assert_eq!(dec("000000.00"), Some(Decimal { value: 0, decimals: 2 }));
    assert_eq!(field::uint(b"000000000", 0), Ok(Some(0)));
}

#[test]
fn only_spread_quotes_are_negative() {
    assert_eq!(dec("-00000.85"), Some(Decimal { value: -85, decimals: 2 }));
    assert_eq!(dec("+00937.05"), Some(Decimal { value: 93705, decimals: 2 }));
}

#[test]
fn an_equity_price_reads_through_the_same_parser() {
    // Eleven bytes: [sign][unused][9 digits]. The unused byte is a '0', so it
    // simply reads as a leading digit.
    assert_eq!(dec("00000074100"), Some(Decimal { value: 74100, decimals: 0 }));
}

#[test]
fn a_bad_sign_byte_is_refused_at_its_offset() {
    assert_eq!(
        field::decimal(b"X00937.05", 154),
        Err(KrxError::Sign { at: 154, found: b'X' })
    );
}

#[test]
fn junk_inside_a_number_is_refused_at_its_offset() {
    assert_eq!(
        field::decimal(b"0009 7.05", 47),
        Err(KrxError::Digit { at: 47 + 4, found: b' ' })
    );
    assert_eq!(field::uint(b"00a", 10), Err(KrxError::Digit { at: 12, found: b'a' }));
}

#[test]
fn a_second_decimal_point_is_refused() {
    assert_eq!(field::decimal(b"00.937.05", 0), Err(KrxError::DecimalPoint { at: 6 }));
}

#[test]
fn a_twenty_two_byte_amount_still_fits_an_i64() {
    // 누적거래대금 is the widest numeric field KRX sends (FLOAT128, 3 decimals).
    let d = dec("000000000155908543.000").unwrap();
    assert_eq!(d.value, 155_908_543_000);
    assert_eq!(d.decimals, 3);
}

#[test]
fn a_number_too_wide_for_the_wire_is_an_error_not_a_wrap() {
    let wide = "0".to_string() + &"9".repeat(30);
    assert!(matches!(field::decimal(wide.as_bytes(), 0), Err(KrxError::Overflow { .. })));
}

#[test]
fn time_of_day_is_nanoseconds_since_midnight() {
    let ns = field::time_of_day_ns(b"090100123456", 35).unwrap().unwrap();
    assert_eq!(ns, (9 * 3600 + 60) * 1_000_000_000 + 123_456_000);

    // Nine bytes is the millisecond form (V1 가격확대시각).
    let ms = field::time_of_day_ns(b"090100123", 33).unwrap().unwrap();
    assert_eq!(ms, (9 * 3600 + 60) * 1_000_000_000 + 123_000_000);
}

#[test]
fn a_blank_time_is_absent() {
    assert_eq!(field::time_of_day_ns(b"            ", 0), Ok(None));
}

#[test]
fn a_clock_reading_that_is_not_one_is_refused() {
    assert_eq!(field::time_of_day_ns(b"250000000000", 35), Err(KrxError::TimeRange { at: 35 }));
    assert_eq!(field::time_of_day_ns(b"096100000000", 35), Err(KrxError::TimeRange { at: 35 }));
    assert_eq!(field::time_of_day_ns(b"090060000000", 35), Err(KrxError::TimeRange { at: 35 }));
    assert_eq!(field::time_of_day_ns(b"0901001234", 35), Err(KrxError::TimeWidth { len: 10 }));
}

#[test]
fn an_isin_is_copied_raw() {
    // Raw, because the interned identifier is process-local and cannot cross
    // the segment (feed_handler.md §5).
    assert_eq!(field::isin(b"KR4101V90009"), Ok(*b"KR4101V90009"));
    assert!(field::isin(b"KR410").is_err());
}
