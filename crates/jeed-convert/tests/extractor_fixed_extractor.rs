use jeed_convert::extractor::{Config, ConfigErr, FixedExtractor, ParseErr};

// ============================================================================
// Unsigned Integer Tests
// ============================================================================

#[test]
fn test_to_u8_integer_only() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(3)
        .with_fraction_size(0)
        .with_is_signed(false);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_u8(b"123"), Ok(123));
    assert_eq!(ext.to_u8(b"001"), Ok(1));
    assert_eq!(ext.to_u8(b"255"), Ok(255));
    Ok(())
}

#[test]
fn test_to_u8_with_decimal() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(1)
        .with_fraction_size(2)
        .with_is_signed(false);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_u8(b"1.23"), Ok(123));
    assert_eq!(ext.to_u8(b"2.55"), Ok(255));
    Ok(())
}

#[test]
fn test_to_u16_integer_only() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(0)
        .with_is_signed(false);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_u16(b"12345"), Ok(12345));
    assert_eq!(ext.to_u16(b"65535"), Ok(65535));
    Ok(())
}

#[test]
fn test_to_u16_with_decimal() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(3)
        .with_fraction_size(2)
        .with_is_signed(false);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_u16(b"123.45"), Ok(12345));
    assert_eq!(ext.to_u16(b"655.35"), Ok(65535));
    Ok(())
}

#[test]
fn test_to_u32_integer_only() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(10)
        .with_fraction_size(0)
        .with_is_signed(false);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_u32(b"4294967295"), Ok(4294967295));
    assert_eq!(ext.to_u32(b"0000000001"), Ok(1));
    Ok(())
}

#[test]
fn test_to_u32_with_decimal() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(false);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_u32(b"12345.67"), Ok(1234567));
    Ok(())
}

#[test]
fn test_to_u64_integer_only() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(20)
        .with_fraction_size(0)
        .with_is_signed(false);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_u64(b"18446744073709551615"), Ok(18446744073709551615));
    Ok(())
}

#[test]
fn test_to_u64_with_decimal() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(8)
        .with_fraction_size(2)
        .with_is_signed(false);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_u64(b"12345678.90"), Ok(1234567890));
    Ok(())
}

#[test]
fn test_to_u128_integer_only() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(10)
        .with_fraction_size(0)
        .with_is_signed(false);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_u128(b"1234567890"), Ok(1234567890));
    Ok(())
}

// ============================================================================
// Signed Integer Tests
// ============================================================================

#[test]
fn test_to_i8_positive() -> Result<(), ConfigErr> {
    // For positive signed numbers without sign prefix, use is_signed=false
    let mut config = Config::builder()
        .with_integer_size(3)
        .with_fraction_size(0)
        .with_is_signed(false);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_i8(b"127"), Ok(127));
    assert_eq!(ext.to_i8(b"001"), Ok(1));
    Ok(())
}

#[test]
fn test_to_i8_negative() -> Result<(), ConfigErr> {
    // For negative numbers, integer_size includes the sign position
    let mut config = Config::builder()
        .with_integer_size(4)  // 1 for sign + 3 for digits
        .with_fraction_size(0)
        .with_is_signed(true);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_i8(b"-128"), Ok(-128));
    assert_eq!(ext.to_i8(b"-001"), Ok(-1));
    Ok(())
}

#[test]
fn test_to_i16_positive() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(0)
        .with_is_signed(false);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_i16(b"32767"), Ok(32767));
    Ok(())
}

#[test]
fn test_to_i16_negative() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(6)  // 1 for sign + 5 for digits
        .with_fraction_size(0)
        .with_is_signed(true);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_i16(b"-32768"), Ok(-32768));
    Ok(())
}

#[test]
fn test_to_i32_positive() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(10)
        .with_fraction_size(0)
        .with_is_signed(false);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_i32(b"2147483647"), Ok(2147483647));
    Ok(())
}

#[test]
fn test_to_i32_negative() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(11)  // 1 for sign + 10 for digits
        .with_fraction_size(0)
        .with_is_signed(true);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_i32(b"-2147483648"), Ok(-2147483648));
    Ok(())
}

#[test]
fn test_to_i64_positive() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(19)
        .with_fraction_size(0)
        .with_is_signed(false);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_i64(b"9223372036854775807"), Ok(9223372036854775807));
    Ok(())
}

#[test]
fn test_to_i64_negative() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(20)  // 1 for sign + 19 for digits
        .with_fraction_size(0)
        .with_is_signed(true);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_i64(b"-9223372036854775808"), Ok(-9223372036854775808));
    Ok(())
}

#[test]
fn test_to_i128_positive() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(10)
        .with_fraction_size(0)
        .with_is_signed(false);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_i128(b"1234567890"), Ok(1234567890));
    Ok(())
}

#[test]
fn test_to_i128_negative() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(11)  // 1 for sign + 10 for digits
        .with_fraction_size(0)
        .with_is_signed(true);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_i128(b"-1234567890"), Ok(-1234567890));
    Ok(())
}

// ============================================================================
// Floating-Point Tests
// ============================================================================

#[test]
fn test_to_f32_basic() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(false);
    config.build()?;
    let ext = FixedExtractor::new(config);

    let result = ext.to_f32(b"12345.67").expect("should parse");
    assert!((result - 12345.67).abs() < 0.01);
    Ok(())
}

#[test]
fn test_to_f32_negative() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(6)  // 1 for sign + 5 for digits
        .with_fraction_size(2)
        .with_is_signed(true);
    config.build()?;
    let ext = FixedExtractor::new(config);

    let result = ext.to_f32(b"-12345.67").expect("should parse");
    assert!((result + 12345.67).abs() < 0.01);
    Ok(())
}

#[test]
fn test_to_f64_basic() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(false);
    config.build()?;
    let ext = FixedExtractor::new(config);

    let result = ext.to_f64(b"12345.67").expect("should parse");
    assert!((result - 12345.67).abs() < 0.0001);
    Ok(())
}

#[test]
fn test_to_f64_negative() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(6)  // 1 for sign + 5 for digits
        .with_fraction_size(2)
        .with_is_signed(true);
    config.build()?;
    let ext = FixedExtractor::new(config);

    let result = ext.to_f64(b"-12345.67").expect("should parse");
    assert!((result + 12345.67).abs() < 0.0001);
    Ok(())
}

#[test]
fn test_to_f64_with_unused_head() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(false)
        .with_unused_head(1);
    config.build()?;
    let ext = FixedExtractor::new(config);

    let result = ext.to_f64(b"x2345.67").expect("should parse");
    assert!((result - 2345.67).abs() < 0.0001);
    Ok(())
}

#[test]
fn test_to_f64_with_unused_tail() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(3)
        .with_is_signed(false)
        .with_unused_tail(1);
    config.build()?;
    let ext = FixedExtractor::new(config);

    // With unused_tail=1, the last digit is ignored
    // "12345.678" -> we read "12345.67" effectively
    let result = ext.to_f64(b"12345.678").expect("should parse");
    assert!((result - 12345.67).abs() < 0.001);
    Ok(())
}

// ============================================================================
// Edge Cases and Error Tests
// ============================================================================

#[test]
fn test_to_u8_overflow() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(3)
        .with_fraction_size(0)
        .with_is_signed(false);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_u8(b"256"), Err(ParseErr::Overflow));
    Ok(())
}

#[test]
fn test_to_i8_overflow() -> Result<(), ConfigErr> {
    // Test positive overflow (128 > i8::MAX=127)
    let mut config = Config::builder()
        .with_integer_size(3)
        .with_fraction_size(0)
        .with_is_signed(false);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_i8(b"128"), Err(ParseErr::Overflow));
    Ok(())
}

#[test]
fn test_to_i8_neg_overflow() -> Result<(), ConfigErr> {
    // Test negative overflow (-129 < i8::MIN=-128)
    let mut config = Config::builder()
        .with_integer_size(4)  // 1 for sign + 3 for digits
        .with_fraction_size(0)
        .with_is_signed(true);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_i8(b"-129"), Err(ParseErr::NegOverflow));
    Ok(())
}

#[test]
fn test_invalid_length() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(false);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_u32(b"123"), Err(ParseErr::InvalidLength));
    Ok(())
}

#[test]
fn test_invalid_digit() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(3)
        .with_fraction_size(0)
        .with_is_signed(false);
    config.build()?;
    let ext = FixedExtractor::new(config);

    assert_eq!(ext.to_u8(b"12a"), Err(ParseErr::InvalidDigit));
    Ok(())
}

// ============================================================================
// Original Tests
// ============================================================================

#[test]
fn test_int_ext_new() {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(true);

    config.build().expect("build error");

    let int_ext = FixedExtractor::new(config.clone());
    assert_eq!(int_ext.config, config);
}

#[test]
fn test_int_ext_clip_basic() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(3)
        .with_fraction_size(2)
        .with_is_signed(false)
        .with_unused_head(1)
        .with_unused_tail(3);

    config.build()?;

    let int_ext = FixedExtractor::new(config);

    let data = b"x23.45";
    let res = int_ext.clip(data).expect("clip error");

    assert_eq!(res, b"23");
    Ok(())
}

#[test]
fn test_int_ext_clip_no_unused_tail() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(3)
        .with_fraction_size(2)
        .with_is_signed(false)
        .with_unused_head(1)
        .with_unused_tail(0);

    config.build()?;

    let int_ext = FixedExtractor::new(config);

    let data = b"x23.45";
    let res = int_ext.clip(data).expect("clip error");

    assert_eq!(res, b"23.45");
    Ok(())
}

#[test]
fn test_int_ext_clip_no_unused() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(false)
        .with_unused_head(0)
        .with_unused_tail(0);

    config.build()?;

    let int_ext = FixedExtractor::new(config);

    let data = b"12345.67";
    let res = int_ext.clip(data).expect("clip error");

    assert_eq!(res, b"12345.67");
    Ok(())
}

#[test]
fn test_int_ext_clip_integer_only() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(0)
        .with_is_signed(false)
        .with_unused_head(0)
        .with_unused_tail(0);

    config.build()?;

    let int_ext = FixedExtractor::new(config);

    let data = b"12345";
    let res = int_ext.clip(data).expect("clip error");

    assert_eq!(res, b"12345");
    Ok(())
}

#[test]
fn test_int_ext_clip_insufficient_data() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(false)
        .with_unused_head(0)
        .with_unused_tail(0);

    config.build()?;

    let int_ext = FixedExtractor::new(config);

    // data is too short (needs 8 bytes: "12345.67")
    let data = b"123";
    let res = int_ext.clip(data);

    assert_eq!(res, Err(ParseErr::InvalidLength));
    Ok(())
}

#[test]
fn test_int_ext_default() {
    let int_ext = FixedExtractor::default();
    assert_eq!(int_ext.config, Config::default());
}

#[test]
fn test_int_ext_clone() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(true);

    config.build()?;

    let int_ext = FixedExtractor::new(config);
    let cloned = int_ext.clone();

    assert_eq!(int_ext, cloned);
    Ok(())
}

#[test]
fn test_int_ext_partial_eq() -> Result<(), ConfigErr> {
    let mut config1 = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(true);
    config1.build()?;

    let mut config2 = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(true);
    config2.build()?;

    let int_ext1 = FixedExtractor::new(config1);
    let int_ext2 = FixedExtractor::new(config2);

    assert_eq!(int_ext1, int_ext2);
    Ok(())
}

#[test]
fn test_int_ext_partial_ord() -> Result<(), ConfigErr> {
    let mut config1 = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(true);
    config1.build()?;

    let mut config2 = Config::builder()
        .with_integer_size(6)
        .with_fraction_size(2)
        .with_is_signed(true);
    config2.build()?;

    let int_ext1 = FixedExtractor::new(config1);
    let int_ext2 = FixedExtractor::new(config2);

    // PartialOrd is derived, so comparison is possible
    assert!(int_ext1 < int_ext2 || int_ext1 >= int_ext2);
    Ok(())
}

#[test]
fn test_int_ext_debug() -> Result<(), ConfigErr> {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(true);
    config.build()?;

    let int_ext = FixedExtractor::new(config);
    let debug_str = format!("{:?}", int_ext);

    assert!(debug_str.contains("FixedExtractor"));
    assert!(debug_str.contains("config"));
    Ok(())
}

