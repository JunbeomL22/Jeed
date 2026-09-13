// Allow useless_vec for test data
#![allow(clippy::useless_vec)]

use jeed_convert::decimal_core::{
    le_bytes_to_u16, le_bytes_to_u32, le_bytes_to_u64, le_bytes_to_u128, squeeze_point_u16,
    squeeze_point_u32, squeeze_point_u64, squeeze_point_u128,
};

#[test]
fn test_squeeze_u16() {
    let test_sets = vec![".1", "1."];

    for (i, test) in test_sets.iter().enumerate() {
        let x: &[u8] = test.as_bytes();
        let x = le_bytes_to_u16(x);

        let val = squeeze_point_u16(x, i).unwrap();
        let val_bytes = val.to_le_bytes();
        let val_utf = std::str::from_utf8(&val_bytes).unwrap();

        assert_eq!(val_utf, "01");
    }
}

#[test]
fn test_squeeze_u32() {
    let u = b"1.23";
    let x: &[u8] = &u[..];
    let x = le_bytes_to_u32(x);

    let val = squeeze_point_u32(x, 1).unwrap();
    let val_bytes = val.to_le_bytes();
    let val_utf = std::str::from_utf8(&val_bytes).unwrap();

    assert_eq!(val_utf, "0123");

    let u = b".123";
    let x: &[u8] = &u[..];
    let x = le_bytes_to_u32(x);

    let val = squeeze_point_u32(x, 0).unwrap();
    let val_bytes = val.to_le_bytes();
    let val_utf = std::str::from_utf8(&val_bytes).unwrap();

    assert_eq!(val_utf, "0123");
}

#[test]
fn test_squeeze_u64() {
    let u = b"12345.67";
    let x: &[u8] = &u[..];
    let x = le_bytes_to_u64(x);

    let val = squeeze_point_u64(x, 5).unwrap();
    let val_bytes = val.to_le_bytes();
    let val_utf = std::str::from_utf8(&val_bytes).unwrap();

    assert_eq!(val_utf, "01234567");

    let u = b".1234567";
    let x: &[u8] = &u[..];
    let x = le_bytes_to_u64(x);

    let val = squeeze_point_u64(x, 0).unwrap();
    let val_bytes = val.to_le_bytes();
    let val_utf = std::str::from_utf8(&val_bytes).unwrap();

    assert_eq!(val_utf, "01234567");
}

#[test]
fn test_squeeze_u128() {
    let u = b"12345.6789012345";
    let x: &[u8] = &u[..];
    let x = le_bytes_to_u128(x);

    let val = squeeze_point_u128(x, 5).unwrap();
    let val_bytes = val.to_le_bytes();
    let val_utf = std::str::from_utf8(&val_bytes).unwrap();

    assert_eq!(val_utf, "0123456789012345");

    let u = b".123456789012345";
    let x: &[u8] = &u[..];
    let x = le_bytes_to_u128(x);

    let val = squeeze_point_u128(x, 0).unwrap();
    let val_bytes = val.to_le_bytes();
    let val_utf = std::str::from_utf8(&val_bytes).unwrap();

    assert_eq!(val_utf, "0123456789012345");
}

