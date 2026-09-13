use jeed_convert::ParseErr;
use jeed_convert::decimal_core::{
    checked_conversion_u8, checked_conversion_u16, checked_conversion_u32, checked_conversion_u64,
    checked_conversion_u128,
};

#[test]
fn test_checked_conversion_u8() {
    for i in 0..10 {
        let input = [b'0' + i];
        assert_eq!(checked_conversion_u8(&input), Ok(i));
    }
    assert_eq!(checked_conversion_u8(b"a"), Err(ParseErr::InvalidDigit));
    assert_eq!(checked_conversion_u8(b" "), Err(ParseErr::InvalidDigit));
}

#[test]
fn test_checked_conversion_u16() {
    assert_eq!(checked_conversion_u16(b"12"), Ok(12));
    assert_eq!(checked_conversion_u16(b"00"), Ok(0));
    assert_eq!(checked_conversion_u16(b"99"), Ok(99));
    assert_eq!(checked_conversion_u16(b"1a"), Err(ParseErr::InvalidDigit));
}

#[test]
fn test_checked_conversion_u32() {
    assert_eq!(checked_conversion_u32(b"1234"), Ok(1234));
    assert_eq!(checked_conversion_u32(b"0000"), Ok(0));
    assert_eq!(checked_conversion_u32(b"123a"), Err(ParseErr::InvalidDigit));
}

#[test]
fn test_checked_conversion_u64() {
    assert_eq!(checked_conversion_u64(b"12345678"), Ok(12345678));
    assert_eq!(checked_conversion_u64(b"00000000"), Ok(0));
    assert_eq!(
        checked_conversion_u64(b"1234567a"),
        Err(ParseErr::InvalidDigit)
    );
}

#[test]
fn test_checked_conversion_u128() {
    assert_eq!(
        checked_conversion_u128(b"1234567890123456"),
        Ok(1234567890123456)
    );
    assert_eq!(checked_conversion_u128(b"0000000000000000"), Ok(0));
    assert_eq!(
        checked_conversion_u128(b"123456789012345a"),
        Err(ParseErr::InvalidDigit)
    );
}

