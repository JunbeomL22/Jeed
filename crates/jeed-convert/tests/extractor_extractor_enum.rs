use jeed_convert::extractor::{
    Config, DynamicExtractor, Extractor, FixedExtractor,
};

#[test]
fn test_extractor_from_fixed() {
    let mut config = Config::default()
        .with_integer_size(3)
        .with_fraction_size(2);
    config.build().expect("Failed to build Config");

    let fixed = FixedExtractor::new(config);
    let extractor: Extractor = fixed.into();

    match extractor {
        Extractor::Fixed(_) => (),
        Extractor::Dynamic(_) => panic!("Expected Fixed variant"),
    }
}

#[test]
fn test_extractor_from_dynamic() {
    let dynamic = DynamicExtractor::new(4);
    let extractor: Extractor = dynamic.into();

    match extractor {
        Extractor::Dynamic(_) => (),
        Extractor::Fixed(_) => panic!("Expected Dynamic variant"),
    }
}

#[test]
fn test_extractor_from_config() {
    let mut config = Config::default()
        .with_integer_size(3)
        .with_fraction_size(2);
    config.build().expect("Failed to build Config");

    let extractor: Extractor = config.into();

    match extractor {
        Extractor::Fixed(_) => (),
        Extractor::Dynamic(_) => panic!("Expected Fixed variant"),
    }
}

#[test]
fn test_extractor_new_fixed() {
    let mut config = Config::default()
        .with_integer_size(3)
        .with_fraction_size(2);
    config.build().expect("Failed to build Config");

    let extractor = Extractor::new_fixed(config);

    match extractor {
        Extractor::Fixed(_) => (),
        Extractor::Dynamic(_) => panic!("Expected Fixed variant"),
    }
}

#[test]
fn test_extractor_new_dynamic() {
    let extractor = Extractor::new_dynamic(4);

    match extractor {
        Extractor::Dynamic(ref d) => assert_eq!(d.target_decimals, 4),
        Extractor::Fixed(_) => panic!("Expected Dynamic variant"),
    }
}

#[test]
fn test_extractor_default() {
    let extractor = Extractor::default();

    match extractor {
        Extractor::Fixed(_) => (),
        Extractor::Dynamic(_) => panic!("Expected Fixed variant as default"),
    }
}

#[test]
fn test_extractor_get_total_size() {
    let mut config = Config::default()
        .with_integer_size(3)
        .with_fraction_size(2);
    config.build().expect("Failed to build Config");

    let fixed_ext = Extractor::new_fixed(config);
    assert!(fixed_ext.get_total_size() > 0);

    let dynamic_ext = Extractor::new_dynamic(4);
    assert_eq!(dynamic_ext.get_total_size(), 0);
}

#[test]
fn test_extractor_dynamic_to_i64() {
    let extractor = Extractor::new_dynamic(2);

    assert_eq!(extractor.to_i64(b"123.45").unwrap(), 12345);
    assert_eq!(extractor.to_i64(b"0.01").unwrap(), 1);
    assert_eq!(extractor.to_i64(b"1000").unwrap(), 100000);
}

#[test]
fn test_extractor_dynamic_to_u64() {
    let extractor = Extractor::new_dynamic(2);

    assert_eq!(extractor.to_u64(b"123.45").unwrap(), 12345);
    assert_eq!(extractor.to_u64(b"0.01").unwrap(), 1);
}

#[test]
fn test_extractor_dynamic_to_f64() {
    let extractor = Extractor::new_dynamic(2);

    let result = extractor.to_f64(b"123.45").unwrap();
    assert!((result - 123.45).abs() < 1e-10);

    let result2 = extractor.to_f64(b"0.01").unwrap();
    assert!((result2 - 0.01).abs() < 1e-10);
}

#[test]
fn test_extractor_dynamic_to_f32() {
    let extractor = Extractor::new_dynamic(2);

    let result = extractor.to_f32(b"123.45").unwrap();
    assert!((result - 123.45).abs() < 1e-5);
}

#[test]
fn test_extractor_fixed_parsing() {
    let mut config = Config::default()
        .with_integer_size(3)
        .with_fraction_size(2);
    config.build().expect("Failed to build Config");

    let extractor = Extractor::new_fixed(config);

    // Fixed extractor parsing test - input needs decimal point for 3+2 config
    let result = extractor.to_i64(b"123.45");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 12345);
}

#[test]
fn test_extractor_clone_and_eq() {
    let extractor1 = Extractor::new_dynamic(4);
    let extractor2 = extractor1.clone();

    assert_eq!(extractor1, extractor2);
}

// === Advisor 피드백 기반 추가 테스트 케이스 ===

#[test]
fn test_extractor_dynamic_to_i128() {
    let extractor = Extractor::new_dynamic(2);
    assert_eq!(extractor.to_i128(b"123.45").unwrap(), 12345);
    assert_eq!(extractor.to_i128(b"0").unwrap(), 0);
    assert_eq!(extractor.to_i128(b"999999999.99").unwrap(), 99999999999);
}

#[test]
fn test_extractor_dynamic_negative() {
    let extractor = Extractor::new_dynamic(2);
    assert_eq!(extractor.to_i64(b"-123.45").unwrap(), -12345);
    assert_eq!(extractor.to_i64(b"-0.01").unwrap(), -1);
    assert_eq!(extractor.to_i64(b"-1000").unwrap(), -100000);
}

#[test]
fn test_extractor_dynamic_u64_negative_overflow() {
    let extractor = Extractor::new_dynamic(2);
    assert!(extractor.to_u64(b"-123.45").is_err());
    assert!(extractor.to_u64(b"-0.01").is_err());
}

#[test]
fn test_extractor_dynamic_empty_input() {
    let extractor = Extractor::new_dynamic(2);
    // Empty input should return InvalidDigit error
    assert!(extractor.to_i64(b"").is_err());
    assert!(extractor.to_u64(b"").is_err());
}

#[test]
fn test_extractor_dynamic_decimal_without_integer() {
    let extractor = Extractor::new_dynamic(2);
    assert_eq!(extractor.to_i64(b".5").unwrap(), 50);
    assert_eq!(extractor.to_i64(b".05").unwrap(), 5);
}

#[test]
fn test_extractor_dynamic_large_numbers() {
    let extractor = Extractor::new_dynamic(2);
    assert_eq!(extractor.to_i64(b"9999999999.99").unwrap(), 999999999999);
    assert_eq!(extractor.to_i128(b"9999999999999.99").unwrap(), 999999999999999);
}

#[test]
fn test_extractor_dynamic_f64_negative() {
    let extractor = Extractor::new_dynamic(2);
    let result = extractor.to_f64(b"-123.45").unwrap();
    assert!((result - (-123.45)).abs() < 1e-10);
}

#[test]
fn test_extractor_dynamic_f32_negative() {
    let extractor = Extractor::new_dynamic(2);
    let result = extractor.to_f32(b"-123.45").unwrap();
    assert!((result - (-123.45)).abs() < 1e-5);
}

#[test]
fn test_extractor_dynamic_high_precision() {
    let extractor = Extractor::new_dynamic(8);
    assert_eq!(extractor.to_i64(b"1.23456789").unwrap(), 123456789);

    let f64_result = extractor.to_f64(b"1.23456789").unwrap();
    assert!((f64_result - 1.23456789).abs() < 1e-10);
}

#[test]
fn test_extractor_dynamic_integer_only() {
    let extractor = Extractor::new_dynamic(2);
    assert_eq!(extractor.to_i64(b"100").unwrap(), 10000);
    assert_eq!(extractor.to_u64(b"100").unwrap(), 10000);

    let f64_result = extractor.to_f64(b"100").unwrap();
    assert!((f64_result - 100.0).abs() < 1e-10);
}

