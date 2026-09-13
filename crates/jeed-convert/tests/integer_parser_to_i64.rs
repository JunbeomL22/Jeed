use jeed_convert::ParseErr;
use jeed_convert::integer_parser::Biscuit;

const I64_LENGTH_BOUND: usize = 20;

#[test]
fn test_back_and_forth() {
    for i in (-1_000_000_i64..1_000_000).step_by(1000) {
        let x = i.to_string();
        let x_byte: &[u8] = x.as_bytes();
        let val = i64::parse_decimal(x_byte);
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
fn test_to_i64() {
    for i in 2..I64_LENGTH_BOUND {
        let mut x_vec: Vec<u8> = vec![b'0'; i];
        x_vec[0] = b'-';
        let x: &[u8] = &x_vec[..];
        let val = i64::parse_decimal(x);
        assert_eq!(
            val,
            Ok(0),
            "Failed for {}-th attempt, where byte: {:?}",
            i,
            x,
        );
    }
    for i in 2..I64_LENGTH_BOUND {
        let mut x_vec: Vec<u8> = vec![b'1'; i];
        x_vec[0] = b'-';
        let x: &[u8] = &x_vec[..];
        let val = i64::parse_decimal(x).unwrap();
        assert_eq!(
            val,
            std::str::from_utf8(x).unwrap().parse::<i64>().unwrap(),
            "Failed for {} bytes",
            i
        );
    }
    for i in 1..(I64_LENGTH_BOUND - 1) {
        let x_vec: Vec<u8> = vec![b'9'; i];
        let x: &[u8] = &x_vec[..];
        let val = i64::parse_decimal(x).unwrap();
        assert_eq!(
            val,
            std::str::from_utf8(x).unwrap().parse::<i64>().unwrap(),
            "Failed for {} bytes",
            i
        );
    }
}

#[test]
fn test_i64_extremes() {
    // Test i64::MAX
    let max_string = i64::MAX.to_string();
    let max_byte: &[u8] = max_string.as_bytes();
    let val = i64::parse_decimal(max_byte);
    assert_eq!(val, Ok(i64::MAX));

    // Test i64::MIN
    let min_string = i64::MIN.to_string();
    let min_byte: &[u8] = min_string.as_bytes();
    let val = i64::parse_decimal(min_byte);
    assert_eq!(val, Ok(i64::MIN));

    // Test Overflow
    let byte_test_p1 = b"9223372036854775808"; // i64::MAX + 1
    let byte_test_n1 = b"-9223372036854775809"; // i64::MIN - 1
    let val_p1 = i64::parse_decimal(byte_test_p1);
    let val_n1 = i64::parse_decimal(byte_test_n1);
    assert_eq!(val_p1, Err(ParseErr::Overflow));
    assert_eq!(val_n1, Err(ParseErr::NegOverflow));
}

#[test]
fn test_i64_leading_zeros() {
    let byte_leading_zeros_pos = b"01234567890123456789";
    let byte_leading_zeros_neg = b"-01234567890123456789";
    let val_leading_zeros_pos = i64::parse_decimal(byte_leading_zeros_pos);
    let val_leading_zeros_neg = i64::parse_decimal(byte_leading_zeros_neg);
    assert_eq!(val_leading_zeros_pos, Ok(1234567890123456789));
    assert_eq!(val_leading_zeros_neg, Ok(-1234567890123456789));
}

