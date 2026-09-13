//! `jeed_krx::field` — fixed-width ASCII readers.

use jeed_convert::ParseErr;
use jeed_krx::KrxError;
use jeed_krx::extract::KRX;
use jeed_krx::field;
use jeed_wire::Scale;

#[test]
fn a_derivative_price_is_read_with_the_shape_its_instrument_uses() {
    // Nine bytes in all three cases. The reader is chosen per instrument, so
    // the value that comes out is already in that instrument's scale.
    assert_eq!(field::price(&KRX.derivative_rate, b"000937.05", 0), Ok(Some(93705)));
    assert_eq!(field::price(&KRX.derivative_plain, b"000012345", 0), Ok(Some(12345)));
    assert_eq!(field::price(&KRX.derivative_risk_free, b"03456.789", 0), Ok(Some(3456789)));
}

#[test]
fn the_scale_belongs_to_the_reader_not_the_value() {
    // This is the whole point of not scanning for the point: the scale is known
    // before the message is read, so it is stamped on the record header once
    // instead of riding along with every price.
    assert_eq!(KRX.derivative_rate.scale(), Some(Scale::S2));
    assert_eq!(KRX.derivative_plain.scale(), Some(Scale::S0));
    assert_eq!(KRX.derivative_risk_free.scale(), Some(Scale::S3));
    assert_eq!(KRX.equity_price.scale(), Some(Scale::S0));
    assert_eq!(KRX.bond_yield.scale(), Some(Scale::S6));
}

#[test]
fn reading_a_price_with_the_wrong_shape_fails_rather_than_shifting_it() {
    // The risk the reader-per-instrument design takes on: pick wrong and a
    // price could come out a hundred times off. It cannot, because each shape
    // expects the '.' at a different index and a digit everywhere else, so a
    // mis-selected reader lands a '.' where a digit belongs.
    let field = b"000937.05"; // [sign][5].[2]
    assert_eq!(field::price(&KRX.derivative_rate, field, 47), Ok(Some(93705)));
    assert!(field::price(&KRX.derivative_plain, field, 47).is_err());
    assert!(field::price(&KRX.derivative_risk_free, field, 47).is_err());

    let plain = b"000012345"; // [sign][8]
    assert_eq!(field::price(&KRX.derivative_plain, plain, 47), Ok(Some(12345)));
    assert!(field::price(&KRX.derivative_rate, plain, 47).is_err());
}

#[test]
fn a_blank_field_is_absent_not_zero() {
    // 정보분배일련번호 arrives blank on whole channels. Reading it as zero makes
    // every message look like a sequence gap (CLAUDE.md).
    assert_eq!(field::price(&KRX.derivative_rate, b"         ", 0), Ok(None));
    assert_eq!(field::uint(b"        ", 0), Ok(None));
    assert!(field::is_blank(b"   "));
    assert!(!field::is_blank(b"  0"));
}

#[test]
fn blank_is_recognised_rather_than_inferred_from_the_parser_failing() {
    // Both are bytes the parser rejects, and they mean opposite things: the
    // first is the exchange saying "not measurable", the second is a corrupt
    // message that has to be dropped.
    assert_eq!(field::uint(b"         ", 0), Ok(None));
    assert_eq!(field::uint(b"0000 0000", 0), Err(KrxError::Field { at: 0, err: ParseErr::InvalidDigit }));
}

#[test]
fn a_zero_field_is_present_and_zero() {
    // The converse of the above, and the reason the dynamic-limit conclusion
    // has to ride in a flag: KRX writes `000000.00`, not blanks, for
    // instruments the regime does not cover.
    assert_eq!(field::price(&KRX.derivative_rate, b"000000.00", 0), Ok(Some(0)));
    assert_eq!(field::uint(b"000000000", 0), Ok(Some(0)));
}

#[test]
fn only_spread_quotes_are_negative() {
    assert_eq!(field::price(&KRX.derivative_rate, b"-00000.85", 0), Ok(Some(-85)));
    assert_eq!(field::price(&KRX.derivative_plain, b"-00000085", 0), Ok(Some(-85)));
}

#[test]
fn an_equity_price_reads_through_its_own_reader() {
    // Eleven bytes: [sign][unused][9 digits]. The unused byte is a '0', so it
    // simply reads as a leading digit.
    assert_eq!(field::price(&KRX.equity_price, b"00000074100", 0), Ok(Some(74100)));
}

#[test]
fn a_bond_yield_keeps_all_six_places() {
    // Thirteen bytes: [sign][5].[6].
    assert_eq!(field::price(&KRX.bond_yield, b"000003.123456", 0), Ok(Some(3123456)));
}

#[test]
fn junk_inside_a_number_is_refused_at_the_fields_offset() {
    assert_eq!(
        field::price(&KRX.derivative_rate, b"0009 7.05", 47),
        Err(KrxError::Field { at: 47, err: ParseErr::InvalidDigit })
    );
    assert_eq!(
        field::uint(b"00a", 10),
        Err(KrxError::Field { at: 10, err: ParseErr::InvalidDigit })
    );
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
    // Each of these folds into a plausible second count, so the check has to
    // happen while the digits are still digits: 096100 would otherwise come out
    // as 10:01:00.
    assert_eq!(field::time_of_day_ns(b"250000000000", 35), Err(KrxError::TimeRange { at: 35 }));
    assert_eq!(field::time_of_day_ns(b"096100000000", 35), Err(KrxError::TimeRange { at: 35 }));
    assert_eq!(field::time_of_day_ns(b"090060000000", 35), Err(KrxError::TimeRange { at: 35 }));
    assert_eq!(
        field::time_of_day_ns(b"0901001234", 35),
        Err(KrxError::Field { at: 35, err: ParseErr::InvalidLength })
    );
}

#[test]
fn the_last_second_of_the_day_is_a_clock_reading() {
    let ns = field::time_of_day_ns(b"235959999999", 0).unwrap().unwrap();
    assert_eq!(ns, 86_399 * 1_000_000_000 + 999_999_000);
    assert!(ns < field::NS_PER_DAY);
}

#[test]
fn an_isin_is_copied_raw() {
    // Raw, because the interned identifier is process-local and cannot cross
    // the segment (feed_handler.md §5).
    assert_eq!(field::isin(b"KR4101V90009"), Ok(*b"KR4101V90009"));
    assert!(field::isin(b"KR410").is_err());
}
