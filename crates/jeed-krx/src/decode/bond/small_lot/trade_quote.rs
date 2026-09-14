//! `G7` — 소액채권 체결 + 우선호가. `IFMSRPD0030`, 1063 B.
//!
//! `A3`'s trade block followed by `IFMSRPD0024`'s book:
//!
//! ```text
//! [0:41]       header (shape B)
//! [41:222]     trade block — byte for byte the block A3 carries
//! [222:1002]   5 × 156 B level block — 종목 78 B then 종류 78 B
//! [1002:1062]  채권·채권종류 매도/매수호가총잔량  ← no wire slot
//! [1062:1063]  0xFF
//! ```

use crate::decode::bond::{self, DEPTH};
use crate::decode::bond::small_lot::{BOOK_TAIL_LEN, LEVEL_LEN};
use crate::decode::common::fill_record_header;
use crate::error::KrxError;
use crate::extract::KRX;
use crate::trcode::TrCode;
use jeed_wire::{
    QuotePayload, RecordHeader, TradeQuotePayload, UnixNano, Venue, WireKind, WireRecord,
};

/// `IFMSRPD0030`.
pub const MESSAGE_LEN: usize = bond::TRADE_BLOCK_END + DEPTH * LEVEL_LEN + BOOK_TAIL_LEN;

const _: () = assert!(MESSAGE_LEN == 1063);

/// The 소액채권 체결+우선호가 decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SmallLotTradeQuote;

/// `IFMSRPD0030`.
pub const DECODER: SmallLotTradeQuote = SmallLotTradeQuote;

impl SmallLotTradeQuote {
    /// Fixed message length this interface defines.
    #[inline]
    pub const fn message_len(&self) -> usize {
        MESSAGE_LEN
    }

    /// Levels per side this interface carries.
    #[inline]
    pub const fn depth(&self) -> usize {
        DEPTH
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

        let mut quote = QuotePayload::default();
        let shape = bond::fill_book(payload, bond::TRADE_BLOCK_END, LEVEL_LEN, &mut quote)?;
        let trade = bond::fill_trade(payload)?;

        let mut h = RecordHeader::new(WireKind::TradeQuote, Venue::Krx, crate::field::wire_symbol(&msg.isin), recv_ns);
        fill_record_header(&mut h, &msg, recv_ns, KRX.bond_price.scale().unwrap_or_default());
        h.set_depth(shape.depth).set_flags(shape.flags);

        *out = WireRecord::new_trade_quote(h, TradeQuotePayload { trade, quote });
        Ok(())
    }
}

/// `true` if this trcode is a `G7` on the 소액채권 channel.
pub const fn handles(trcode: TrCode) -> bool {
    matches!(trcode.data_class(), [b'G', b'7']) && bond::is_small_lot_group(trcode)
}
