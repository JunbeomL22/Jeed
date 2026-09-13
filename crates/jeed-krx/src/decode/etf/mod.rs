//! ETF·ELW·ETN·수익증권 — the 증권 products that have a liquidity provider.
//!
//! | 상품군 | |
//! |---|---|
//! | `02S` | ELW |
//! | `03S` | ETF |
//! | `04S` | ETN |
//! | `05S` | 상장형 수익증권 |
//!
//! All four take the `B7` 우선호가 form, which is the `B6` book plus the LP's
//! share of each level. 체결 is the same interface 주식 uses.

pub mod quote;
pub mod trade;

use crate::trcode::TrCode;

/// `true` for the four LP-bearing 증권 product groups.
#[inline]
pub const fn is_lp_group(trcode: TrCode) -> bool {
    matches!(
        trcode.product_group(),
        [b'0', b'2', b'S'] | [b'0', b'3', b'S'] | [b'0', b'4', b'S'] | [b'0', b'5', b'S']
    )
}
