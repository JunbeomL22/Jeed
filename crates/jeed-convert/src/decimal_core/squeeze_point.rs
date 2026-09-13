//! Decimal point removal from packed ASCII digit representations.

use crate::ParseErr;

const POINT_LOCATION_MASK_U16: [u16; 2] = [0xFF00, 0x00FF];
const POINT_LOCATION_VALUE_U16: [u16; 2] = [0x2E00, 0x002E];
const DECIMAL_MASK_U16: [u16; 2] = [0x0000, 0x00FF];

/// Removes a decimal point from a packed `u16` ASCII representation.
///
/// # Errors
/// Returns [`ParseErr::InvalidPointIndex`] if index >= 2.
/// Returns [`ParseErr::InvalidPointLocation`] if no decimal point at index.
#[inline]
pub fn squeeze_point_u16(chunk: u16, numeric_point_idx: usize) -> Result<u16, ParseErr> {
    if numeric_point_idx >= 2 {
        return Err(ParseErr::InvalidPointIndex);
    }
    if chunk & POINT_LOCATION_MASK_U16[numeric_point_idx]
        == POINT_LOCATION_VALUE_U16[numeric_point_idx]
    {
        return Err(ParseErr::InvalidPointLocation);
    }

    let mut res = chunk;
    let decimal_mask = DECIMAL_MASK_U16[numeric_point_idx];
    let fraction_mask = !decimal_mask << 8;
    let decimal_part = (res & decimal_mask) << 8;
    let fraction_part = res & fraction_mask;
    res = fraction_part + decimal_part + 0x0030;

    Ok(res)
}

const POINT_LOCATION_MASK_U32: [u32; 4] = [0xFF00_0000, 0x00FF_0000, 0x0000_FF00, 0x0000_00FF];
const POINT_LOCATION_VALUE_U32: [u32; 4] = [0x2E00_0000, 0x002E_0000, 0x0000_2E00, 0x0000_002E];
const DECIMAL_MASK_U32: [u32; 4] = [0x0000_0000, 0x0000_00FF, 0x0000_FFFF, 0x00FF_FFFF];

/// Removes a decimal point from a packed `u32` ASCII representation.
///
/// # Errors
/// Returns [`ParseErr::InvalidPointIndex`] if index >= 4.
/// Returns [`ParseErr::InvalidPointLocation`] if no decimal point at index.
#[inline]
pub fn squeeze_point_u32(chunk: u32, numeric_point_idx: usize) -> Result<u32, ParseErr> {
    if numeric_point_idx >= 4 {
        return Err(ParseErr::InvalidPointIndex);
    }
    if chunk & POINT_LOCATION_MASK_U32[numeric_point_idx]
        == POINT_LOCATION_VALUE_U32[numeric_point_idx]
    {
        return Err(ParseErr::InvalidPointLocation);
    }

    let mut res = chunk;
    let decimal_mask = DECIMAL_MASK_U32[numeric_point_idx];
    let fraction_mask = !decimal_mask << 8;
    let decimal_part = (res & decimal_mask) << 8;
    let fraction_part = res & fraction_mask;
    res = fraction_part + decimal_part + 0x0000_0030;

    Ok(res)
}

const POINT_LOCATION_MASK_U64: [u64; 8] = [
    0xFF00_0000_0000_0000,
    0x00FF_0000_0000_0000,
    0x0000_FF00_0000_0000,
    0x0000_00FF_0000_0000,
    0x0000_0000_FF00_0000,
    0x0000_0000_00FF_0000,
    0x0000_0000_0000_FF00,
    0x0000_0000_0000_00FF,
];

const POINT_LOCATION_VALUE_U64: [u64; 8] = [
    0x2E00_0000_0000_0000,
    0x002E_0000_0000_0000,
    0x0000_2E00_0000_0000,
    0x0000_002E_0000_0000,
    0x0000_0000_2E00_0000,
    0x0000_0000_002E_0000,
    0x0000_0000_0000_2E00,
    0x0000_0000_0000_002E,
];

const DECIMAL_MASK_U64: [u64; 8] = [
    0x0000_0000_0000_0000,
    0x0000_0000_0000_00FF,
    0x0000_0000_0000_FFFF,
    0x0000_0000_00FF_FFFF,
    0x0000_0000_FFFF_FFFF,
    0x0000_00FF_FFFF_FFFF,
    0x0000_FFFF_FFFF_FFFF,
    0x00FF_FFFF_FFFF_FFFF,
];

/// Removes a decimal point from a packed `u64` ASCII representation.
///
/// # Errors
/// Returns [`ParseErr::InvalidPointIndex`] if index >= 8.
/// Returns [`ParseErr::InvalidPointLocation`] if no decimal point at index.
#[inline]
pub fn squeeze_point_u64(chunk: u64, numeric_point_idx: usize) -> Result<u64, ParseErr> {
    if numeric_point_idx >= 8 {
        return Err(ParseErr::InvalidPointIndex);
    }
    if chunk & POINT_LOCATION_MASK_U64[numeric_point_idx]
        == POINT_LOCATION_VALUE_U64[numeric_point_idx]
    {
        return Err(ParseErr::InvalidPointLocation);
    }

    let mut res = chunk;
    let decimal_mask = DECIMAL_MASK_U64[numeric_point_idx];
    let fraction_mask = !decimal_mask << 8;
    let decimal_part = (res & decimal_mask) << 8;
    let fraction_part = res & fraction_mask;
    res = fraction_part + decimal_part + 0x0000_0000_0000_0030;

    Ok(res)
}

const POINT_LOCATION_MASK_U128: [u128; 16] = [
    0xFF00_0000_0000_0000_0000_0000_0000_0000,
    0x00FF_0000_0000_0000_0000_0000_0000_0000,
    0x0000_FF00_0000_0000_0000_0000_0000_0000,
    0x0000_00FF_0000_0000_0000_0000_0000_0000,
    0x0000_0000_FF00_0000_0000_0000_0000_0000,
    0x0000_0000_00FF_0000_0000_0000_0000_0000,
    0x0000_0000_0000_FF00_0000_0000_0000_0000,
    0x0000_0000_0000_00FF_0000_0000_0000_0000,
    0x0000_0000_0000_0000_FF00_0000_0000_0000,
    0x0000_0000_0000_0000_00FF_0000_0000_0000,
    0x0000_0000_0000_0000_0000_FF00_0000_0000,
    0x0000_0000_0000_0000_0000_00FF_0000_0000,
    0x0000_0000_0000_0000_0000_0000_FF00_0000,
    0x0000_0000_0000_0000_0000_0000_00FF_0000,
    0x0000_0000_0000_0000_0000_0000_0000_FF00,
    0x0000_0000_0000_0000_0000_0000_0000_00FF,
];

const POINT_LOCATION_VALUE_U128: [u128; 16] = [
    0x2E00_0000_0000_0000_0000_0000_0000_0000,
    0x002E_0000_0000_0000_0000_0000_0000_0000,
    0x0000_2E00_0000_0000_0000_0000_0000_0000,
    0x0000_002E_0000_0000_0000_0000_0000_0000,
    0x0000_0000_2E00_0000_0000_0000_0000_0000,
    0x0000_0000_002E_0000_0000_0000_0000_0000,
    0x0000_0000_0000_2E00_0000_0000_0000_0000,
    0x0000_0000_0000_002E_0000_0000_0000_0000,
    0x0000_0000_0000_0000_2E00_0000_0000_0000,
    0x0000_0000_0000_0000_002E_0000_0000_0000,
    0x0000_0000_0000_0000_0000_2E00_0000_0000,
    0x0000_0000_0000_0000_0000_002E_0000_0000,
    0x0000_0000_0000_0000_0000_0000_2E00_0000,
    0x0000_0000_0000_0000_0000_0000_002E_0000,
    0x0000_0000_0000_0000_0000_0000_0000_2E00,
    0x0000_0000_0000_0000_0000_0000_0000_002E,
];

const DECIMAL_MASK_U128: [u128; 16] = [
    0x0000_0000_0000_0000_0000_0000_0000_0000,
    0x0000_0000_0000_0000_0000_0000_0000_00FF,
    0x0000_0000_0000_0000_0000_0000_0000_FFFF,
    0x0000_0000_0000_0000_0000_0000_00FF_FFFF,
    0x0000_0000_0000_0000_0000_0000_FFFF_FFFF,
    0x0000_0000_0000_0000_0000_00FF_FFFF_FFFF,
    0x0000_0000_0000_0000_0000_FFFF_FFFF_FFFF,
    0x0000_0000_0000_0000_00FF_FFFF_FFFF_FFFF,
    0x0000_0000_0000_0000_FFFF_FFFF_FFFF_FFFF,
    0x0000_0000_0000_00FF_FFFF_FFFF_FFFF_FFFF,
    0x0000_0000_0000_FFFF_FFFF_FFFF_FFFF_FFFF,
    0x0000_0000_00FF_FFFF_FFFF_FFFF_FFFF_FFFF,
    0x0000_0000_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF,
    0x0000_00FF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF,
    0x0000_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF,
    0x00FF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF,
];

/// Removes a decimal point from a packed `u128` ASCII representation.
///
/// # Errors
/// Returns [`ParseErr::InvalidPointIndex`] if index >= 16.
/// Returns [`ParseErr::InvalidPointLocation`] if no decimal point at index.
#[inline]
pub fn squeeze_point_u128(chunk: u128, numeric_point_idx: usize) -> Result<u128, ParseErr> {
    if numeric_point_idx >= 16 {
        return Err(ParseErr::InvalidPointIndex);
    }
    if chunk & POINT_LOCATION_MASK_U128[numeric_point_idx]
        == POINT_LOCATION_VALUE_U128[numeric_point_idx]
    {
        return Err(ParseErr::InvalidPointLocation);
    }

    let decimal_mask = DECIMAL_MASK_U128[numeric_point_idx];
    let fraction_mask = !decimal_mask << 8;
    let decimal_part = (chunk & decimal_mask) << 8;
    let fraction_part = chunk & fraction_mask;
    let res = fraction_part + decimal_part + 0x0000_0000_0000_0000_0000_0000_0000_0030u128;

    Ok(res)
}
