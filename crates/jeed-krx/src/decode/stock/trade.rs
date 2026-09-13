//! `A3` — 주식 체결.
//!
//! KRX sends **one** 체결 interface for the whole 증권 market: `IFMSRPD0004`
//! covers 주식 and the LP products alike. So this is
//! [`securities::trade`](crate::decode::securities::trade) under the name the market
//! module gives it, rather than a second copy of the same offsets waiting to
//! drift out of step with the first.

pub use crate::decode::securities::trade::{DECODER, SecuritiesTrade, MESSAGE_LEN};

use crate::trcode::TrCode;

/// `true` if this trcode is an `A3` on a 주식 channel.
pub const fn handles(trcode: TrCode) -> bool {
    matches!(trcode.data_class(), [b'A', b'3']) && super::is_stock_group(trcode)
}
