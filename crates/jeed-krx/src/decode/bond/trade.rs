//! `A3` — 채권 체결. `IFMSRPD0027`, 223 B.
//!
//! ```text
//! [0:41]     header (shape B)
//! [41:52]    체결가격          → TradePayload::price
//! [52:62]    거래량 (천원)     → TradePayload::qty
//! [62:70]    거래일자          ← no wire slot
//! [70:92]    거래대금          ← no wire slot
//! [92:105]   체결수익률        → TradePayload::trade_yield
//! [105:138]  시가 / 고가 / 저가      ← no wire slot
//! [138:177]  시가·고가·저가 수익률   ← no wire slot
//! [177:192]  채권누적체결수량  → TradePayload::cumulative_qty
//! [192:214]  누적거래대금      ← no wire slot
//! [214:222]  결제일자          ← no wire slot
//! [222:223]  0xFF
//! ```
//!
//! Note the message covers `01M` 소액채권 too, but that market's 우선호가 and
//! 체결+우선호가 forms are different interfaces this build does not decode, so
//! only `01B`/`01K` are claimed here — a half-decoded market is worse than an
//! undecoded one.

use crate::decode::bond;
use crate::decode::common::fill_record_header;
use crate::error::KrxError;
use crate::extract::KRX;
use crate::trcode::TrCode;
use jeed_wire::{RecordHeader, UnixNano, Venue, WireKind, WireRecord};

/// `IFMSRPD0027`.
pub const MESSAGE_LEN: usize = bond::TRADE_BLOCK_END + 1;

const _: () = assert!(MESSAGE_LEN == 223);

/// The 채권 체결 decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BondTrade;

/// `IFMSRPD0027`.
pub const DECODER: BondTrade = BondTrade;

impl BondTrade {
    /// Fixed message length this interface defines.
    #[inline]
    pub const fn message_len(&self) -> usize {
        MESSAGE_LEN
    }

    /// Decodes into `out`, leaving it untouched on error.
    pub fn decode(
        &self,
        payload: &[u8],
        recv_ns: UnixNano,
        out: &mut WireRecord,
    ) -> Result<(), KrxError> {
        crate::message::validate(payload, MESSAGE_LEN)?;
        let msg = bond::header(payload)?;
        let trade = bond::fill_trade(payload)?;

        let mut h = RecordHeader::new(WireKind::Trade, Venue::Krx, msg.isin, recv_ns);
        fill_record_header(&mut h, &msg, recv_ns, KRX.bond_price.scale().unwrap_or_default());

        *out = WireRecord::new_trade(h, trade);
        Ok(())
    }
}

/// `true` if this trcode is an `A3` on a 일반채권·국고채권 channel.
pub const fn handles(trcode: TrCode) -> bool {
    matches!(trcode.data_class(), [b'A', b'3']) && bond::is_bond_group(trcode)
}
