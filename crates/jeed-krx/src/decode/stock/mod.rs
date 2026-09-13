//! 주식 — 유가증권 `01S`, 코스닥 `01Q`, 코넥스 `01X`.
//!
//! The market with no liquidity provider, so its 우선호가 is the `B6` form
//! without LP quantities. Everything else it shares with
//! [`etf`](super::etf) lives in [`securities`](super::securities).

pub mod quote;
pub mod trade;

use crate::trcode::TrCode;

/// `true` for the three 주식 product groups.
///
/// `02S`–`05S` are **not** here: ELW, ETF, ETN and 수익증권 have liquidity
/// providers and take the `B7` form instead ([`etf`](super::etf)).
#[inline]
pub const fn is_stock_group(trcode: TrCode) -> bool {
    matches!(
        trcode.product_group(),
        [b'0', b'1', b'S'] | [b'0', b'1', b'Q'] | [b'0', b'1', b'X']
    )
}
