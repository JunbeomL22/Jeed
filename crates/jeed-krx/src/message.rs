//! The frame check every decoder runs before it reads a single field.

use crate::error::KrxError;
use crate::trcode::{TRCODE_LEN, TrCode};

/// 정보분배메세지종료키워드 — the last byte of every KRX message.
pub const END_KEYWORD: u8 = 0xFF;

/// Checks that a datagram is the length its interface defines and ends with
/// the end keyword.
///
/// This is the same position FIX's `BodyLength` + `CheckSum` occupy: two
/// independent statements that the frame is whole, checked **before** any
/// field is read. Reading fields first and validating after means a truncated
/// datagram gets parsed into plausible-looking numbers on the way to the
/// failure.
#[inline]
pub const fn validate(payload: &[u8], expected_len: usize) -> Result<(), KrxError> {
    if payload.len() != expected_len {
        return Err(KrxError::Length { expected: expected_len, actual: payload.len() });
    }
    if expected_len == 0 {
        return Err(KrxError::TooShort { need: TRCODE_LEN + 1, got: 0 });
    }
    let found = payload[expected_len - 1];
    if found != END_KEYWORD {
        return Err(KrxError::EndKeyword { found });
    }
    Ok(())
}

/// Checks the frame **and** that it carries the trcode the caller dispatched on.
///
/// The receive loop reads the trcode to pick a decoder, so the decoder
/// re-reading it is not redundant work — it is the decoder refusing to apply
/// one interface's layout to another interface's bytes.
#[inline]
pub const fn validate_as(
    payload: &[u8],
    expected: TrCode,
    expected_len: usize,
) -> Result<(), KrxError> {
    if let Err(e) = validate(payload, expected_len) {
        return Err(e);
    }
    let found = match TrCode::from_message(payload) {
        Ok(c) => c,
        Err(e) => return Err(e),
    };
    if found.as_u64() != expected.as_u64() {
        return Err(KrxError::UnknownTrCode { code: found });
    }
    Ok(())
}
