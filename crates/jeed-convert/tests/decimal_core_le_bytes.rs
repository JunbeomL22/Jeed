use jeed_convert::decimal_core::{
    le_bytes_to_u16, le_bytes_to_u32, le_bytes_to_u64, le_bytes_to_u128,
};

#[test]
fn test_le_bytes_to_u16() {
    let u = b"12";
    let result = le_bytes_to_u16(u);
    assert_eq!(result.to_le_bytes(), *b"12");
}

#[test]
fn test_le_bytes_to_u32() {
    let u = b"1234";
    let result = le_bytes_to_u32(u);
    assert_eq!(result.to_le_bytes(), *b"1234");
}

#[test]
fn test_le_bytes_to_u64() {
    let u = b"12345678";
    let result = le_bytes_to_u64(u);
    assert_eq!(result.to_le_bytes(), *b"12345678");
}

#[test]
fn test_le_bytes_to_u128() {
    let u = b"1234567890123456";
    let result = le_bytes_to_u128(u);
    assert_eq!(result.to_le_bytes(), *b"1234567890123456");
}

