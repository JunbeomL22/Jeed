use jeed_convert::ParseErr;
use jeed_convert::integer_parser::Biscuit;

#[test]
fn test_back_and_forth() {
    for i in (-1_000_000_i128..1_000_000).step_by(1000) {
        let x = i.to_string();
        let x_byte: &[u8] = x.as_bytes();
        let val = i128::parse_decimal(x_byte);
        assert_eq!(
            val,
            Ok(i),
            "Failed for {} the string: \"{}\" byte: {:?}",
            i,
            x,
            x_byte,
        );
    }
}

#[test]
fn test_i128_extremes() {
    // Test i128::MAX
    let max_string = i128::MAX.to_string();
    let max_byte: &[u8] = max_string.as_bytes();
    let val = i128::parse_decimal(max_byte);
    assert_eq!(val, Ok(i128::MAX));

    // Test i128::MIN
    let min_string = i128::MIN.to_string();
    let min_byte: &[u8] = min_string.as_bytes();
    let val = i128::parse_decimal(min_byte);
    assert_eq!(val, Ok(i128::MIN));

    // Test Overflow
    let byte_test_p1 = b"170141183460469231731687303715884105728"; // i128::MAX + 1
    let byte_test_n1 = b"-170141183460469231731687303715884105729"; // i128::MIN - 1
    let val_p1 = i128::parse_decimal(byte_test_p1);
    let val_n1 = i128::parse_decimal(byte_test_n1);
    assert_eq!(val_p1, Err(ParseErr::Overflow));
    assert_eq!(val_n1, Err(ParseErr::NegOverflow));
}

#[test]
fn test_i128_leading_zeros() {
    let byte_leading_zeros_pos = b"000000000000000000000000000000000001234567890123456789";
    let byte_leading_zeros_neg = b"-000000000000000000000000000000000001234567890123456789";
    let val_leading_zeros_pos = i128::parse_decimal(byte_leading_zeros_pos);
    let val_leading_zeros_neg = i128::parse_decimal(byte_leading_zeros_neg);
    assert_eq!(val_leading_zeros_pos, Ok(1234567890123456789));
    assert_eq!(val_leading_zeros_neg, Ok(-1234567890123456789));
}

