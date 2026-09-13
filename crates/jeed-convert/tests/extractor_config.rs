use jeed_convert::extractor::Config;

#[test]
fn test_config_builder_basic() {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(true)
        .with_unused_head(0)
        .with_unused_tail(0);

    config.build().expect("build error");

    assert_eq!(config.integer_size, 5);
    assert_eq!(config.fraction_size, 2);
    assert!(config.is_signed);
    assert_eq!(config.unused_head, 0);
    assert_eq!(config.unused_tail, 0);
}

#[test]
fn test_config_total_size_signed_with_fraction() {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(true);

    config.build().expect("build error");

    // total_size = integer_size + 1 (sign) + 1 (decimal point) + fraction_size
    // = 5 + 1 + 1 + 2 = 9
    assert_eq!(config.total_size, 9);
}

#[test]
fn test_config_total_size_unsigned_no_fraction() {
    let mut config = Config::builder()
        .with_integer_size(7)
        .with_fraction_size(0)
        .with_is_signed(false);

    config.build().expect("build error");

    // total_size = integer_size = 7 (no sign, no fraction)
    assert_eq!(config.total_size, 7);
}

#[test]
fn test_config_numeric_point_idx() {
    let mut config = Config::builder()
        .with_integer_size(4)
        .with_fraction_size(4)
        .with_is_signed(true)
        .with_unused_head(0);

    config.build().expect("build error");

    // numeric_point_idx = integer_size - unused_head = 4 - 0 = 4
    assert_eq!(config.numeric_point_idx, Some(4));
}

#[test]
fn test_config_no_decimal_point_when_no_fraction() {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(0)
        .with_is_signed(false);

    config.build().expect("build error");

    assert_eq!(config.numeric_point_idx, None);
}

#[test]
fn test_config_used_numeric_length() {
    let mut config = Config::builder()
        .with_integer_size(4)
        .with_fraction_size(4)
        .with_is_signed(true)
        .with_unused_head(0)
        .with_unused_tail(0);

    config.build().expect("build error");

    // used_numeric_length = integer_size + 1 (decimal) + fraction_size - unused_head - unused_tail
    // = 4 + 1 + 4 - 0 - 0 = 9
    assert_eq!(config.used_numeric_length, 9);
}

#[test]
fn test_config_with_unused_head() {
    let mut config = Config::builder()
        .with_integer_size(7)
        .with_fraction_size(2)
        .with_is_signed(true)
        .with_unused_head(1)
        .with_unused_tail(0);

    config.build().expect("build error");

    assert_eq!(config.numeric_start_idx, 1);
    // numeric_point_idx = integer_size - numeric_start_idx = 7 - 1 = 6
    assert_eq!(config.numeric_point_idx, Some(6));
}

#[test]
fn test_config_fraction_divisor() {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(3)
        .with_is_signed(false)
        .with_unused_head(0)
        .with_unused_tail(0);

    config.build().expect("build error");

    // fraction_divisor = 10^(fraction_size - unused_tail) = 10^3 = 1000
    assert_eq!(config.fraction_divisor, 1000);
    assert!((config.fraction_multiplier - 0.001).abs() < f64::EPSILON);
}

#[test]
fn test_config_fraction_divisor_with_unused_tail() {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(3)
        .with_is_signed(false)
        .with_unused_head(0)
        .with_unused_tail(1);

    config.build().expect("build error");

    // fraction_divisor = 10^(fraction_size - unused_tail) = 10^2 = 100
    assert_eq!(config.fraction_divisor, 100);
    assert!((config.fraction_multiplier - 0.01).abs() < f64::EPSILON);
}

#[test]
fn test_config_default() {
    let config = Config::default();

    assert_eq!(config.total_size, 0);
    assert_eq!(config.numeric_point_idx, None);
    assert_eq!(config.fraction_size, 0);
    assert_eq!(config.integer_size, 0);
    assert!(!config.is_signed);
    assert_eq!(config.unused_head, 0);
    assert_eq!(config.unused_tail, 0);
}

#[test]
fn test_config_numeric_indices() {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(true)
        .with_unused_head(1)
        .with_unused_tail(0);

    config.build().expect("build error");

    assert_eq!(config.numeric_start_idx, 1);
    // numeric_end_idx = numeric_start_idx + used_numeric_length
    // used_numeric_length = 5 + 1 + 2 - 1 - 0 = 7
    assert_eq!(config.used_numeric_length, 7);
    assert_eq!(config.numeric_end_idx, 8);
}

// === 추가 테스트 케이스 ===

#[test]
fn test_config_with_numeric_point_idx_manual() {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(false)
        .with_numeric_point_idx(Some(3));

    config.build().expect("build error");

    // build()가 numeric_point_idx를 재계산하므로 수동 설정값은 덮어씌워짐
    // fraction_size > 0이면 자동 계산됨
    assert!(config.numeric_point_idx.is_some());
}

#[test]
fn test_config_no_numeric_point_idx_when_no_fraction() {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(0)
        .with_is_signed(false)
        .with_numeric_point_idx(Some(3)); // 수동 설정 시도

    config.build().expect("build error");

    // fraction_size가 0이면 numeric_point_idx는 None
    assert_eq!(config.numeric_point_idx, None);
}

#[test]
fn test_config_clone() {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(true);

    config.build().expect("build error");

    let cloned = config.clone();
    assert_eq!(config, cloned);
}

#[test]
fn test_config_partial_ord() {
    let mut config1 = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(false);
    config1.build().expect("build error");

    let mut config2 = Config::builder()
        .with_integer_size(6)
        .with_fraction_size(2)
        .with_is_signed(false);
    config2.build().expect("build error");

    // total_size가 다르므로 비교 가능
    assert!(config1 < config2);
}

#[test]
fn test_config_debug() {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(true);

    config.build().expect("build error");

    let debug_str = format!("{:?}", config);
    assert!(debug_str.contains("Config"));
    assert!(debug_str.contains("total_size"));
    assert!(debug_str.contains("integer_size"));
}

#[test]
fn test_config_fraction_factors_zero_fraction() {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(0)
        .with_is_signed(false);

    config.build().expect("build error");

    // fraction_size=0이면 fraction_divisor=1, fraction_multiplier=1.0
    assert_eq!(config.fraction_divisor, 1);
    assert!((config.fraction_multiplier - 1.0).abs() < f64::EPSILON);
}

#[test]
fn test_config_large_fraction_size() {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(8)
        .with_is_signed(false);

    config.build().expect("build error");

    // fraction_divisor = 10^8 = 100000000
    assert_eq!(config.fraction_divisor, 100000000);
    assert!((config.fraction_multiplier - 0.00000001).abs() < 1e-15);
}

#[test]
fn test_config_unused_tail_larger_than_fraction() {
    let mut config = Config::builder()
        .with_integer_size(5)
        .with_fraction_size(2)
        .with_is_signed(false)
        .with_unused_tail(3); // unused_tail > fraction_size

    config.build().expect("build error");

    // saturating_sub로 인해 exponent = 0이 됨
    // fraction_divisor = 10^0 = 1
    assert_eq!(config.fraction_divisor, 1);
}

#[test]
fn test_config_both_unused_head_and_tail() {
    let mut config = Config::builder()
        .with_integer_size(7)
        .with_fraction_size(4)
        .with_is_signed(false)
        .with_unused_head(2)
        .with_unused_tail(2);

    config.build().expect("build error");

    // total_size = 7 + 0 + 1 + 4 = 12
    assert_eq!(config.total_size, 12);
    // numeric_start_idx = unused_head = 2
    assert_eq!(config.numeric_start_idx, 2);
    // used_numeric_length = 7 + 1 + 4 - 2 - 2 = 8
    assert_eq!(config.used_numeric_length, 8);
    // numeric_end_idx = 2 + 8 = 10
    assert_eq!(config.numeric_end_idx, 10);
}

#[test]
fn test_config_signed_total_size_calculation() {
    let mut config = Config::builder()
        .with_integer_size(3)
        .with_fraction_size(0)
        .with_is_signed(true);

    config.build().expect("build error");

    // total_size = integer_size + sign(1) + 0 (no fraction) = 3 + 1 = 4
    assert_eq!(config.total_size, 4);
}

#[test]
fn test_config_unsigned_with_fraction_total_size() {
    let mut config = Config::builder()
        .with_integer_size(3)
        .with_fraction_size(2)
        .with_is_signed(false);

    config.build().expect("build error");

    // total_size = integer_size + 0 (no sign) + 1 (decimal) + fraction_size = 3 + 1 + 2 = 6
    assert_eq!(config.total_size, 6);
}

#[test]
fn test_config_chained_builder() {
    let mut config = Config::builder()
        .with_integer_size(10)
        .with_fraction_size(5)
        .with_is_signed(true)
        .with_unused_head(1)
        .with_unused_tail(1);

    config.build().expect("build error");

    assert_eq!(config.integer_size, 10);
    assert_eq!(config.fraction_size, 5);
    assert!(config.is_signed);
    assert_eq!(config.unused_head, 1);
    assert_eq!(config.unused_tail, 1);
}

