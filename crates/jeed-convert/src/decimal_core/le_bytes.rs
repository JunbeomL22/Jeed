//! Little-endian byte slice to integer conversion.

/// Converts a byte slice to `u128` using little-endian order.
///
/// Pads with zeros on the left if input is shorter than 16 bytes.
///
/// # Panics
/// Panics if input length exceeds 16 bytes.
#[inline]
#[must_use]
pub fn le_bytes_to_u128(input: &[u8]) -> u128 {
    assert!(input.len() <= 16, "input length exceeds 16 bytes");
    let mut bytes = [0u8; 16];
    let start = 16 - input.len();
    bytes[start..].copy_from_slice(input);
    u128::from_le_bytes(bytes)
}

/// Converts a byte slice to `u64` using little-endian order.
///
/// Pads with zeros on the left if input is shorter than 8 bytes.
///
/// # Panics
/// Panics if input length exceeds 8 bytes.
#[inline]
#[must_use]
pub fn le_bytes_to_u64(input: &[u8]) -> u64 {
    assert!(input.len() <= 8, "input length exceeds 8 bytes");
    let mut bytes = [0u8; 8];
    let start = 8 - input.len();
    bytes[start..].copy_from_slice(input);
    u64::from_le_bytes(bytes)
}

/// Converts a byte slice to `u32` using little-endian order.
///
/// Pads with zeros on the left if input is shorter than 4 bytes.
///
/// # Panics
/// Panics if input length exceeds 4 bytes.
#[inline]
#[must_use]
pub fn le_bytes_to_u32(input: &[u8]) -> u32 {
    assert!(input.len() <= 4, "input length exceeds 4 bytes");
    let mut bytes = [0u8; 4];
    let start = 4 - input.len();
    bytes[start..].copy_from_slice(input);
    u32::from_le_bytes(bytes)
}

/// Converts a byte slice to `u16` using little-endian order.
///
/// Pads with zeros on the left if input is shorter than 2 bytes.
///
/// # Panics
/// Panics if input length exceeds 2 bytes.
#[inline]
#[must_use]
pub fn le_bytes_to_u16(input: &[u8]) -> u16 {
    assert!(input.len() <= 2, "input length exceeds 2 bytes");
    let mut bytes = [0u8; 2];
    let start = 2 - input.len();
    bytes[start..].copy_from_slice(input);
    u16::from_le_bytes(bytes)
}
