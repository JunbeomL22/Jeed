//! Signed integer decimal parsing implementations.

use super::Biscuit;
use crate::ParseErr;

impl Biscuit for i128 {
    #[inline]
    fn parse_decimal(u: &[u8]) -> Result<Self, ParseErr> {
        if !u.is_empty() && u[0] == b'-' {
            u128::unsigned_decimal_core(&u[1..], true, false)
                .map(|val| (!(val as i128)).wrapping_add(1))
        } else {
            u128::unsigned_decimal_core(u, false, true).map(|val| val as i128)
        }
    }
}

impl Biscuit for i64 {
    #[inline]
    fn parse_decimal(u: &[u8]) -> Result<Self, ParseErr> {
        if !u.is_empty() && u[0] == b'-' {
            u64::unsigned_decimal_core(&u[1..], true, false)
                .map(|val| (!(val as i64)).wrapping_add(1))
        } else {
            u64::unsigned_decimal_core(u, false, true).map(|val| val as i64)
        }
    }
}

impl Biscuit for i32 {
    #[inline]
    fn parse_decimal(u: &[u8]) -> Result<Self, ParseErr> {
        if !u.is_empty() && u[0] == b'-' {
            u32::unsigned_decimal_core(&u[1..], true, false)
                .map(|val| (!(val as i32)).wrapping_add(1))
        } else {
            u32::unsigned_decimal_core(u, false, true).map(|val| val as i32)
        }
    }
}

impl Biscuit for i16 {
    #[inline]
    fn parse_decimal(u: &[u8]) -> Result<Self, ParseErr> {
        if !u.is_empty() && u[0] == b'-' {
            u16::unsigned_decimal_core(&u[1..], true, false)
                .map(|val| (!(val as i16)).wrapping_add(1))
        } else {
            u16::unsigned_decimal_core(u, false, true).map(|val| val as i16)
        }
    }
}

impl Biscuit for i8 {
    #[inline]
    fn parse_decimal(u: &[u8]) -> Result<Self, ParseErr> {
        if !u.is_empty() && u[0] == b'-' {
            u8::unsigned_decimal_core(&u[1..], true, false)
                .map(|val| (!(val as i8)).wrapping_add(1))
        } else {
            u8::unsigned_decimal_core(u, false, true).map(|val| val as i8)
        }
    }
}
