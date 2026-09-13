//! "Have I seen it?" bitmasks.
//!
//! A mask rather than an `Option` per field: the check is one comparison at the
//! end of the walk instead of one per field, and it makes *the venue stopped
//! sending this* a decode failure rather than a zero on the wire — which for a
//! price is the difference between a dropped frame and a free trade.
//!
//! Each exchange module owns its own `seen` constants, because the bits mean
//! different keys on each. What is shared is the check.

use crate::error::CryptoError;

/// `Ok` when every bit of `need` is set in `have`, else the first missing
/// field named by `keys`, which pairs each bit with the key it stands for.
#[inline]
pub(crate) fn require(have: u8, need: u8, keys: &[(u8, &'static str)]) -> Result<(), CryptoError> {
    if have & need == need {
        return Ok(());
    }
    for &(bit, key) in keys {
        if need & bit != 0 && have & bit == 0 {
            return Err(CryptoError::Missing { key });
        }
    }
    // Unreachable while `keys` covers `need`; an incomplete table must still
    // not let a half-read frame through.
    Err(CryptoError::Empty)
}
