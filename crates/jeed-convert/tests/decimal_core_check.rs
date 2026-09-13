// Allow manual is_ascii_check as the alternative may be less clear
#![allow(clippy::manual_is_ascii_check)]

use jeed_convert::decimal_core::{
    check_decimal_bit_u16_optimal, check_decimal_bit_u32_optimal, check_decimal_bit_u64_optimal,
    check_decimal_bit_u128_optimal, le_bytes_to_u16, le_bytes_to_u32, le_bytes_to_u64,
    le_bytes_to_u128,
};

fn check_decimal(input: &[u8]) -> bool {
    input.iter().all(|&x| (b'0'..=b'9').contains(&x))
}

#[test]
fn test_check_decimal_u16_optimal() {
    for i in 0..100 {
        let u = format!("{:02}", i);
        let u = u.as_bytes();
        let chunk = le_bytes_to_u16(u);
        assert_eq!(check_decimal_bit_u16_optimal(chunk), check_decimal(u));
    }

    let invalid_cases = [b"1x", b"x1", b"ab", b"zy", b"!@", b"  "];
    for u in invalid_cases.iter() {
        let chunk = le_bytes_to_u16(*u);
        assert!(!check_decimal_bit_u16_optimal(chunk));
    }
}

#[test]
fn test_check_decimal_u32_optimal() {
    for i in 0..10000 {
        let u = format!("{:04}", i);
        let u = u.as_bytes();
        let chunk = le_bytes_to_u32(u);
        assert_eq!(check_decimal_bit_u32_optimal(chunk), check_decimal(u));
    }

    let invalid_cases = [b"1x1x", b"x11a", b"ab11", b"zyab", b"!@#$", b"    "];
    for u in invalid_cases.iter() {
        let chunk = le_bytes_to_u32(*u);
        assert!(!check_decimal_bit_u32_optimal(chunk));
    }
}

#[test]
fn test_check_decimal_u64_optimal() {
    for i in 0..10000 {
        let u = format!("{:08}", i);
        let u = u.as_bytes();
        let chunk = le_bytes_to_u64(u);
        assert_eq!(check_decimal_bit_u64_optimal(chunk), check_decimal(u));
    }

    let invalid_cases = [
        b"1x1x1x1x",
        b"x11a11ax",
        b"ab11ab11",
        b"yabzyabz",
        b"!@#$%^&*",
        b"        ",
    ];
    for u in invalid_cases.iter() {
        let chunk = le_bytes_to_u64(*u);
        assert!(!check_decimal_bit_u64_optimal(chunk));
    }
}

#[test]
fn test_check_decimal_u128_optimal() {
    for i in 0..10000 {
        let u = format!("{:016}", i);
        let u = u.as_bytes();
        let chunk = le_bytes_to_u128(u);
        assert_eq!(check_decimal_bit_u128_optimal(chunk), check_decimal(u));
    }

    let invalid_cases = [
        b"1x1x1x1x1x1x1x1x",
        b"11111a11a1x1x1x1",
        b"ab11ab11a1x1x1x1",
        b"zyabzyabz1x1x1x1",
    ];
    for u in invalid_cases.iter() {
        let chunk = le_bytes_to_u128(*u);
        assert!(!check_decimal_bit_u128_optimal(chunk));
    }
}

#[test]
fn test_space_rejection() {
    let spaces = b"        ";
    let data = le_bytes_to_u64(spaces);
    assert!(!check_decimal_bit_u64_optimal(data));

    let bytes = b"1234123x";
    let data = le_bytes_to_u64(bytes);
    assert!(!check_decimal_bit_u64_optimal(data));

    let bytes = b"12341234";
    let data = le_bytes_to_u64(bytes);
    assert!(check_decimal_bit_u64_optimal(data));
}

