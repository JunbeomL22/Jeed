use jeed_convert::decimal_core::{
    eight_to_u64, four_to_u32, le_bytes_to_u16, le_bytes_to_u32, le_bytes_to_u64, le_bytes_to_u128,
    sixteen_to_u128, two_to_u16_decimal,
};

#[test]
fn test_two_to_u16_decimal() {
    let u = b"12";
    let x = le_bytes_to_u16(u);
    assert_eq!(two_to_u16_decimal(x), 12);
}

#[test]
fn test_four_to_u32() {
    let u = b"1234";
    let x = le_bytes_to_u32(u);
    assert_eq!(four_to_u32(x), 1234);
}

#[test]
fn test_eight_to_u64() {
    let u = b"12345678";
    let x = le_bytes_to_u64(u);
    assert_eq!(eight_to_u64(x), 12345678);
}

#[test]
fn test_sixteen_to_u128() {
    let u = b"1234567890123456";
    let x = le_bytes_to_u128(u);
    assert_eq!(sixteen_to_u128(x), 1234567890123456);
}

