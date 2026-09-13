use jeed_convert::ParseErr;
use jeed_convert::extractor::DynamicExtractor;

#[test]
fn test_basic_decimal_parsing() {
    // 소수점 있는 경우
    let ext2 = DynamicExtractor::new(2);
    let ext4 = DynamicExtractor::new(4);

    assert_eq!(ext2.parse(b"123.45"), 12345);
    assert_eq!(ext4.parse(b"123.45"), 1234500);
    assert_eq!(ext2.parse(b"123.4567"), 12345);
    assert_eq!(ext4.parse(b"123.4567"), 1234567);
}

#[test]
fn test_no_decimal_point() {
    // 소수점 없는 경우
    let ext0 = DynamicExtractor::new(0);
    let ext2 = DynamicExtractor::new(2);
    let ext4 = DynamicExtractor::new(4);

    assert_eq!(ext0.parse(b"123"), 123);
    assert_eq!(ext2.parse(b"123"), 12300);
    assert_eq!(ext4.parse(b"123"), 1230000);
}

#[test]
fn test_zero_target_decimals() {
    // target_decimals가 0인 경우
    let ext0 = DynamicExtractor::new(0);

    assert_eq!(ext0.parse(b"123.45"), 123);
    assert_eq!(ext0.parse(b"123.99"), 123);
}

#[test]
fn test_exact_decimal_places() {
    // 소수 자리수가 target_decimals와 같은 경우
    let ext2 = DynamicExtractor::new(2);
    let ext1 = DynamicExtractor::new(1);
    let ext3 = DynamicExtractor::new(3);

    assert_eq!(ext2.parse(b"123.45"), 12345);
    assert_eq!(ext1.parse(b"1.5"), 15);
    assert_eq!(ext3.parse(b"0.123"), 123);
}

#[test]
fn test_fewer_decimal_places() {
    // 소수 자리수가 target_decimals보다 적은 경우
    let ext2 = DynamicExtractor::new(2);
    let ext3 = DynamicExtractor::new(3);

    assert_eq!(ext2.parse(b"123.4"), 12340);
    assert_eq!(ext3.parse(b"123.4"), 123400);
    assert_eq!(ext3.parse(b"1.5"), 1500);
}

#[test]
fn test_more_decimal_places() {
    // 소수 자리수가 target_decimals보다 많은 경우 (truncation)
    let ext2 = DynamicExtractor::new(2);
    let ext3 = DynamicExtractor::new(3);

    assert_eq!(ext2.parse(b"123.456"), 12345);
    assert_eq!(ext3.parse(b"123.4567"), 123456);
    assert_eq!(ext2.parse(b"1.99999"), 199);
}

#[test]
fn test_zero_values() {
    let ext2 = DynamicExtractor::new(2);

    assert_eq!(ext2.parse(b"0"), 0);
    assert_eq!(ext2.parse(b"0.0"), 0);
    assert_eq!(ext2.parse(b"0.00"), 0);
}

#[test]
fn test_small_values() {
    let ext2 = DynamicExtractor::new(2);
    let ext3 = DynamicExtractor::new(3);

    assert_eq!(ext2.parse(b"0.01"), 1);
    assert_eq!(ext3.parse(b"0.001"), 1);
    assert_eq!(ext2.parse(b"0.1"), 10);
}

#[test]
fn test_large_values() {
    let ext2 = DynamicExtractor::new(2);

    assert_eq!(ext2.parse(b"999999.99"), 99999999);
    assert_eq!(ext2.parse(b"1000000.00"), 100000000);
}

#[test]
fn test_new_and_default() {
    let ext_new = DynamicExtractor::new(4);
    let ext_default = DynamicExtractor::default();

    assert_eq!(ext_new.target_decimals, 4);
    assert_eq!(ext_default.target_decimals, 0);
}

#[test]
fn test_reusability() {
    // extractor 재사용 테스트
    let ext = DynamicExtractor::new(2);

    // 동일한 extractor로 여러 값 파싱
    assert_eq!(ext.parse(b"100.00"), 10000);
    assert_eq!(ext.parse(b"200.50"), 20050);
    assert_eq!(ext.parse(b"0.01"), 1);
    assert_eq!(ext.parse(b"999.99"), 99999);
}

// === Advisor 피드백 기반 추가 테스트 케이스 ===

#[test]
fn test_negative_values() {
    let ext = DynamicExtractor::new(2);
    assert_eq!(ext.parse(b"-123.45"), -12345);
    assert_eq!(ext.parse(b"-0.01"), -1);
    assert_eq!(ext.parse(b"-1000"), -100000);
}

#[test]
fn test_negative_zero_handling() {
    let ext = DynamicExtractor::new(2);
    assert_eq!(ext.parse(b"-0"), 0);
    assert_eq!(ext.parse(b"-0.0"), 0);
    assert_eq!(ext.parse(b"-0.00"), 0);
}

#[test]
fn test_empty_input() {
    let ext = DynamicExtractor::new(2);
    assert_eq!(ext.parse(b""), 0);
}

#[test]
fn test_integer_zero() {
    let ext = DynamicExtractor::new(2);
    assert_eq!(ext.parse(b"0"), 0);
}

#[test]
fn test_integer_only() {
    let ext = DynamicExtractor::new(2);
    assert_eq!(ext.parse(b"123"), 12300);
}

#[test]
fn test_decimal_without_integer_part() {
    let ext = DynamicExtractor::new(2);
    assert_eq!(ext.parse(b".5"), 50);
    assert_eq!(ext.parse(b".05"), 5);
    assert_eq!(ext.parse(b".123"), 12); // truncated
}

#[test]
fn test_large_numbers() {
    let ext = DynamicExtractor::new(2);
    assert_eq!(ext.parse(b"9999999999.99"), 999999999999);
}

#[test]
fn test_very_large_numbers() {
    let ext = DynamicExtractor::new(2);
    assert_eq!(ext.parse(b"123456789012.34"), 12345678901234);
}

#[test]
fn test_negative_large_numbers() {
    let ext = DynamicExtractor::new(2);
    assert_eq!(ext.parse(b"-9999999999.99"), -999999999999);
}

#[test]
fn test_single_digit_decimal() {
    let ext = DynamicExtractor::new(2);
    assert_eq!(ext.parse(b"1.2"), 120);
    assert_eq!(ext.parse(b"0.1"), 10);
}

#[test]
fn test_many_decimal_places_truncation() {
    let ext = DynamicExtractor::new(2);
    // 소수점 이하 많은 자릿수 (truncation)
    assert_eq!(ext.parse(b"1.23456789"), 123);
}

#[test]
fn test_high_precision_target() {
    let ext = DynamicExtractor::new(8);
    assert_eq!(ext.parse(b"1.23456789"), 123456789);
    assert_eq!(ext.parse(b"0.00000001"), 1);
}

#[test]
fn test_negative_with_many_decimals() {
    let ext = DynamicExtractor::new(4);
    assert_eq!(ext.parse(b"-123.4567"), -1234567);
    assert_eq!(ext.parse(b"-0.0001"), -1);
}

// === '+' 부호 지원 테스트 ===

#[test]
fn test_positive_sign_basic() {
    let ext = DynamicExtractor::new(2);
    assert_eq!(ext.parse(b"+123.45"), 12345);
    assert_eq!(ext.parse(b"+0.01"), 1);
    assert_eq!(ext.parse(b"+1000"), 100000);
}

#[test]
fn test_positive_sign_zero() {
    let ext = DynamicExtractor::new(2);
    assert_eq!(ext.parse(b"+0"), 0);
    assert_eq!(ext.parse(b"+0.0"), 0);
    assert_eq!(ext.parse(b"+0.00"), 0);
}

#[test]
fn test_positive_sign_decimal_only() {
    let ext = DynamicExtractor::new(2);
    assert_eq!(ext.parse(b"+.5"), 50);
    assert_eq!(ext.parse(b"+.05"), 5);
}

// === try_parse() 메서드 테스트 ===

#[test]
fn test_try_parse_valid_inputs() {
    let ext = DynamicExtractor::new(2);
    assert_eq!(ext.try_parse(b"123.45"), Some(12345));
    assert_eq!(ext.try_parse(b"-123.45"), Some(-12345));
    assert_eq!(ext.try_parse(b"+123.45"), Some(12345));
    assert_eq!(ext.try_parse(b"123"), Some(12300));
    assert_eq!(ext.try_parse(b"0"), Some(0));
    assert_eq!(ext.try_parse(b"0.01"), Some(1));
}

#[test]
fn test_try_parse_invalid_inputs() {
    let ext = DynamicExtractor::new(2);
    assert_eq!(ext.try_parse(b""), None);  // empty
    assert_eq!(ext.try_parse(b"abc"), None);  // non-numeric
    assert_eq!(ext.try_parse(b"12.34.56"), None);  // multiple decimal points
    assert_eq!(ext.try_parse(b"12a34"), None);  // mixed
    assert_eq!(ext.try_parse(b"-"), None);  // sign only
    assert_eq!(ext.try_parse(b"+"), None);  // sign only
    assert_eq!(ext.try_parse(b"."), None);  // decimal point only
}

#[test]
fn test_try_parse_edge_cases() {
    let ext = DynamicExtractor::new(2);
    assert_eq!(ext.try_parse(b"-.5"), Some(-50));  // negative decimal only
    assert_eq!(ext.try_parse(b"+.5"), Some(50));  // positive decimal only
    assert_eq!(ext.try_parse(b"-0"), Some(0));  // negative zero
    assert_eq!(ext.try_parse(b"+0"), Some(0));  // positive zero
}

#[test]
fn test_try_parse_whitespace_rejected() {
    let ext = DynamicExtractor::new(2);
    assert_eq!(ext.try_parse(b" 123"), None);  // leading space
    assert_eq!(ext.try_parse(b"123 "), None);  // trailing space
    assert_eq!(ext.try_parse(b"12 34"), None);  // embedded space
}

#[test]
fn test_try_parse_special_chars_rejected() {
    let ext = DynamicExtractor::new(2);
    assert_eq!(ext.try_parse(b"1,234.56"), None);  // comma
    assert_eq!(ext.try_parse(b"$123.45"), None);  // currency
    assert_eq!(ext.try_parse(b"123.45%"), None);  // percent
}

// === 경계값 테스트 (i64::MAX, i64::MIN) ===

#[test]
fn test_boundary_i64_max() {
    // i64::MAX = 9223372036854775807
    let ext = DynamicExtractor::new(0);
    assert_eq!(ext.parse(b"9223372036854775807"), i64::MAX);
}

#[test]
fn test_boundary_i64_min_magnitude() {
    // i64::MIN = -9223372036854775808
    // Note: i64::MIN의 절대값은 i64로 표현할 수 없으므로,
    // 여기서는 음수 파싱이 올바르게 동작하는지 확인
    let ext = DynamicExtractor::new(0);
    // -9223372036854775807 (i64::MIN + 1)
    assert_eq!(ext.parse(b"-9223372036854775807"), -9223372036854775807);
}

#[test]
fn test_boundary_large_with_decimals() {
    // 큰 수와 소수점 조합
    let ext = DynamicExtractor::new(2);
    // 92233720368547758.07 -> 9223372036854775807
    assert_eq!(ext.parse(b"92233720368547758.07"), 9223372036854775807);
}

#[test]
fn test_try_parse_boundary_values() {
    let ext = DynamicExtractor::new(0);
    assert_eq!(ext.try_parse(b"9223372036854775807"), Some(i64::MAX));
    assert_eq!(ext.try_parse(b"-9223372036854775807"), Some(-9223372036854775807));
}

// === to_uXX / to_iXX 메서드 테스트 ===

#[test]
fn test_to_u8_valid() {
    let ext = DynamicExtractor::new(0);
    assert_eq!(ext.to_u8(b"0").unwrap(), 0);
    assert_eq!(ext.to_u8(b"255").unwrap(), 255);
    assert_eq!(ext.to_u8(b"128").unwrap(), 128);
}

#[test]
fn test_to_u8_overflow() {
    let ext = DynamicExtractor::new(0);
    assert!(ext.to_u8(b"256").is_err());
    assert!(ext.to_u8(b"1000").is_err());
}

#[test]
fn test_to_u8_negative_overflow() {
    let ext = DynamicExtractor::new(0);
    assert!(ext.to_u8(b"-1").is_err());
    assert!(ext.to_u8(b"-100").is_err());
}

#[test]
fn test_to_u16_valid() {
    let ext = DynamicExtractor::new(0);
    assert_eq!(ext.to_u16(b"0").unwrap(), 0);
    assert_eq!(ext.to_u16(b"65535").unwrap(), 65535);
    assert_eq!(ext.to_u16(b"32768").unwrap(), 32768);
}

#[test]
fn test_to_u16_overflow() {
    let ext = DynamicExtractor::new(0);
    assert!(ext.to_u16(b"65536").is_err());
    assert!(ext.to_u16(b"100000").is_err());
}

#[test]
fn test_to_u32_valid() {
    let ext = DynamicExtractor::new(0);
    assert_eq!(ext.to_u32(b"0").unwrap(), 0);
    assert_eq!(ext.to_u32(b"4294967295").unwrap(), 4294967295);
}

#[test]
fn test_to_u32_overflow() {
    let ext = DynamicExtractor::new(0);
    assert!(ext.to_u32(b"4294967296").is_err());
}

#[test]
fn test_to_u128_valid() {
    let ext = DynamicExtractor::new(0);
    assert_eq!(ext.to_u128(b"0").unwrap(), 0);
    assert_eq!(ext.to_u128(b"9999999999999").unwrap(), 9999999999999);
}

#[test]
fn test_to_i8_valid() {
    let ext = DynamicExtractor::new(0);
    assert_eq!(ext.to_i8(b"0").unwrap(), 0);
    assert_eq!(ext.to_i8(b"127").unwrap(), 127);
    assert_eq!(ext.to_i8(b"-128").unwrap(), -128);
}

#[test]
fn test_to_i8_overflow() {
    let ext = DynamicExtractor::new(0);
    assert!(ext.to_i8(b"128").is_err());
    assert!(ext.to_i8(b"-129").is_err());
}

#[test]
fn test_to_i16_valid() {
    let ext = DynamicExtractor::new(0);
    assert_eq!(ext.to_i16(b"0").unwrap(), 0);
    assert_eq!(ext.to_i16(b"32767").unwrap(), 32767);
    assert_eq!(ext.to_i16(b"-32768").unwrap(), -32768);
}

#[test]
fn test_to_i16_overflow() {
    let ext = DynamicExtractor::new(0);
    assert!(ext.to_i16(b"32768").is_err());
    assert!(ext.to_i16(b"-32769").is_err());
}

#[test]
fn test_to_i32_valid() {
    let ext = DynamicExtractor::new(0);
    assert_eq!(ext.to_i32(b"0").unwrap(), 0);
    assert_eq!(ext.to_i32(b"2147483647").unwrap(), 2147483647);
    assert_eq!(ext.to_i32(b"-2147483648").unwrap(), -2147483648);
}

#[test]
fn test_to_i32_overflow() {
    let ext = DynamicExtractor::new(0);
    assert!(ext.to_i32(b"2147483648").is_err());
    assert!(ext.to_i32(b"-2147483649").is_err());
}

#[test]
fn test_to_i128_valid() {
    let ext = DynamicExtractor::new(0);
    assert_eq!(ext.to_i128(b"0").unwrap(), 0);
    assert_eq!(ext.to_i128(b"9999999999999").unwrap(), 9999999999999);
    assert_eq!(ext.to_i128(b"-9999999999999").unwrap(), -9999999999999);
}

// === 소수점이 있는 경우의 to_uXX / to_iXX 테스트 ===

#[test]
fn test_to_u8_with_decimals() {
    let ext = DynamicExtractor::new(2);
    // "1.23" -> 123 (target_decimals=2)
    assert_eq!(ext.to_u8(b"1.23").unwrap(), 123);
    // "2.55" -> 255
    assert_eq!(ext.to_u8(b"2.55").unwrap(), 255);
}

#[test]
fn test_to_u16_with_decimals() {
    let ext = DynamicExtractor::new(2);
    // "123.45" -> 12345
    assert_eq!(ext.to_u16(b"123.45").unwrap(), 12345);
    // "655.35" -> 65535
    assert_eq!(ext.to_u16(b"655.35").unwrap(), 65535);
}

#[test]
fn test_to_i8_with_decimals() {
    let ext = DynamicExtractor::new(1);
    // "12.7" -> 127
    assert_eq!(ext.to_i8(b"12.7").unwrap(), 127);
    // "-12.8" -> -128
    assert_eq!(ext.to_i8(b"-12.8").unwrap(), -128);
}

#[test]
fn test_to_i16_with_decimals() {
    let ext = DynamicExtractor::new(2);
    // "327.67" -> 32767
    assert_eq!(ext.to_i16(b"327.67").unwrap(), 32767);
    // "-327.68" -> -32768
    assert_eq!(ext.to_i16(b"-327.68").unwrap(), -32768);
}

// === f32/f64 정밀도 테스트 ===

#[test]
fn test_to_f32_precision() {
    let ext = DynamicExtractor::new(4);
    let result = ext.to_f32(b"123.4567").unwrap();
    assert!((result - 123.4567).abs() < 1e-4);
}

#[test]
fn test_to_f64_precision() {
    let ext = DynamicExtractor::new(8);
    let result = ext.to_f64(b"123.45678901").unwrap();
    assert!((result - 123.45678901).abs() < 1e-8);
}

#[test]
fn test_to_f64_zero() {
    let ext = DynamicExtractor::new(2);
    let result = ext.to_f64(b"0").unwrap();
    assert_eq!(result, 0.0);
}

#[test]
fn test_to_f32_zero() {
    let ext = DynamicExtractor::new(2);
    let result = ext.to_f32(b"0.00").unwrap();
    assert_eq!(result, 0.0);
}

// === 유효하지 않은 입력 테스트 ===

#[test]
fn test_to_u64_invalid_input() {
    let ext = DynamicExtractor::new(0);
    assert!(ext.to_u64(b"abc").is_err());
    assert!(ext.to_u64(b"12.34.56").is_err());
    assert!(ext.to_u64(b"").is_err());
}

#[test]
fn test_to_i64_invalid_input() {
    let ext = DynamicExtractor::new(0);
    assert!(ext.to_i64(b"abc").is_err());
    assert!(ext.to_i64(b"12.34.56").is_err());
    assert!(ext.to_i64(b"").is_err());
}

#[test]
fn test_to_f64_invalid_input() {
    let ext = DynamicExtractor::new(2);
    assert!(ext.to_f64(b"abc").is_err());
    assert!(ext.to_f64(b"").is_err());
}

// === Serde 테스트 ===

// === Clone, PartialEq, PartialOrd 테스트 ===

#[test]
fn test_dynamic_extractor_clone() {
    let ext1 = DynamicExtractor::new(4);
    let ext2 = ext1.clone();
    assert_eq!(ext1, ext2);
}

#[test]
fn test_dynamic_extractor_partial_ord() {
    let ext1 = DynamicExtractor::new(2);
    let ext2 = DynamicExtractor::new(4);
    assert!(ext1 < ext2);
}

#[test]
fn test_dynamic_extractor_debug() {
    let ext = DynamicExtractor::new(3);
    let debug_str = format!("{:?}", ext);
    assert!(debug_str.contains("DynamicExtractor"));
    assert!(debug_str.contains("target_decimals"));
}


// ============================================================================
// to_i64_exact / to_u64_exact — the readers that refuse to round
// ============================================================================

#[test]
fn exact_keeps_what_the_plain_reader_truncates() {
    let e = DynamicExtractor::new(2);
    // The plain reader is right for a fixed-width field and wrong for venue
    // text: a truncated price is still a plausible-looking price.
    assert_eq!(e.to_i64(b"0.000015"), Ok(0));
    assert_eq!(e.to_i64_exact(b"0.000015"), Err(ParseErr::Precision));
}

#[test]
fn exact_accepts_the_trailing_zeros_crypto_venues_pad_with() {
    // Binance sends every size at the symbol's full precision, so the digits
    // past a shorter configured scale are normally zeros.
    let e = DynamicExtractor::new(5);
    assert_eq!(e.to_u64_exact(b"3.84410000"), Ok(384_410));
    assert_eq!(e.to_i64_exact(b"4712.06000000"), Ok(471_206_000));
}

#[test]
fn exact_accepts_fewer_decimals_than_the_scale() {
    let e = DynamicExtractor::new(4);
    assert_eq!(e.to_i64_exact(b"25.35"), Ok(253_500));
    assert_eq!(e.to_i64_exact(b"25"), Ok(250_000));
}

#[test]
fn exact_keeps_the_sign() {
    let e = DynamicExtractor::new(2);
    assert_eq!(e.to_i64_exact(b"-12.3400"), Ok(-1_234));
    assert_eq!(e.to_u64_exact(b"-12.34"), Err(ParseErr::NegOverflow));
}

#[test]
fn exact_refuses_a_digit_string_that_would_wrap_the_accumulator() {
    // try_parse folds into an i64 with no overflow check, so the guard is the
    // digit count rather than the value.
    let e = DynamicExtractor::new(2);
    assert_eq!(e.to_i64_exact(b"12345678901234567890.00"), Err(ParseErr::Overflow));
    assert_eq!(e.to_i64_exact(b"123456789012345.00"), Ok(12_345_678_901_234_500));
}

#[test]
fn exact_rejects_what_is_not_a_number() {
    let e = DynamicExtractor::new(2);
    assert_eq!(e.to_i64_exact(b""), Err(ParseErr::Empty));
    assert_eq!(e.to_i64_exact(b"abc"), Err(ParseErr::InvalidDigit));
    assert_eq!(e.to_i64_exact(b"1.2.3"), Err(ParseErr::InvalidDigit));
}
