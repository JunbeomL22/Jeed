//! `A3` — ETF·ELW·ETN 체결.
//!
//! The same `IFMSRPD0004` 주식 uses — see
//! [`securities::trade`](crate::decode::securities::trade). The one field that is
//! specific to these products, LP보유수량 at `[148:163]`, has no wire slot; it
//! matters for ETN inventory rather than for pricing, and it can be negative.

pub use crate::decode::securities::trade::{DECODER, SecuritiesTrade, MESSAGE_LEN};

use crate::trcode::TrCode;

/// `true` if this trcode is an `A3` on an ETF·ELW·ETN·수익증권 channel.
pub const fn handles(trcode: TrCode) -> bool {
    matches!(trcode.data_class(), [b'A', b'3']) && super::is_lp_group(trcode)
}
