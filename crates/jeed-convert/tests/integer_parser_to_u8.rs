use jeed_convert::ParseErr;
use jeed_convert::integer_parser::Biscuit;

#[test]
fn test_back_and_forth() {
    for i in 0_u8..=u8::MAX {
        let x = i.to_string();
        let x_byte: &[u8] = x.as_bytes();
        let val = u8::parse_decimal(x_byte);
        assert_eq!(val, Ok(i), "Failed for {} bytes", i);
    }
}

#[test]
fn test_u8_max() {
    let max_string = u8::MAX.to_string();
    let max_byte: &[u8] = max_string.as_bytes();
    let val = u8::parse_decimal(max_byte);
    assert_eq!(val, Ok(u8::MAX));

    let byte_test_p1 = b"256"; // u8::MAX + 1
    let val_p1 = u8::parse_decimal(byte_test_p1);
    assert_eq!(val_p1, Err(ParseErr::Overflow));
}

#[test]
fn test_u8_leading_zeros() {
    let byte_leading_zeros = b"0123";
    let x_leading_zeros: &[u8] = &byte_leading_zeros[..];
    let val_leading_zeros = u8::parse_decimal(x_leading_zeros);
    assert_eq!(val_leading_zeros, Ok(123));
}

#[test]
fn test_u8_various() {
    assert_eq!(u8::parse_decimal(b"0"), Ok(0));
    assert_eq!(u8::parse_decimal(b"1"), Ok(1));
    assert_eq!(u8::parse_decimal(b"12"), Ok(12));
    assert_eq!(u8::parse_decimal(b"123"), Ok(123));
    assert_eq!(u8::parse_decimal(b"255"), Ok(255));
    assert_eq!(u8::parse_decimal(b""), Err(ParseErr::Empty));
    assert_eq!(u8::parse_decimal(b"abc"), Err(ParseErr::InvalidDigit));
}

