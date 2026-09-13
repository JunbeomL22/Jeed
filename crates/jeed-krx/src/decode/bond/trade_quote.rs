//! `G7` — 일반채권·국고채권 체결 + 우선호가. `IFMSRPD0029`, 643 B.
//!
//! `A3`'s trade block followed by `B6`'s book:
//!
//! ```text
//! [0:41]     header (shape B)
//! [41:222]   trade block — byte for byte the block A3 carries
//! [222:612]  5 × 78 B level block
//! [612:642]  채권매도/매수호가총잔량  ← no wire slot
//! [642:643]  0xFF
//! ```
//!
//! As on the derivative side, this is the form to consume where it is sent: the
//! print and the book it left behind cannot be separated by loss or reordering.

use crate::decode::bond::{self, DEPTH, LEVEL_LEN};
use crate::decode::common::fill_record_header;
use crate::error::KrxError;
use crate::extract::KRX;
use crate::trcode::TrCode;
use jeed_wire::{
    QuotePayload, RecordHeader, TradeQuotePayload, UnixNano, Venue, WireKind, WireRecord,
};

/// `IFMSRPD0029`.
pub const MESSAGE_LEN: usize = bond::TRADE_BLOCK_END + DEPTH * LEVEL_LEN + bond::BOOK_TAIL_LEN;

const _: () = assert!(MESSAGE_LEN == 643);

/// The 채권 체결+우선호가 decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BondTradeQuote;

/// `IFMSRPD0029`.
pub const DECODER: BondTradeQuote = BondTradeQuote;

impl BondTradeQuote {
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
        let shape = bond::fill_book(payload, bond::TRADE_BLOCK_END, &mut quote)?;
        let trade = bond::fill_trade(payload)?;

        let mut h = RecordHeader::new(WireKind::TradeQuote, Venue::Krx, crate::field::wire_symbol(&msg.isin), recv_ns);
        fill_record_header(&mut h, &msg, recv_ns, KRX.bond_price.scale().unwrap_or_default());
        h.set_depth(shape.depth).set_flags(shape.flags);

        *out = WireRecord::new_trade_quote(h, TradeQuotePayload { trade, quote });
        Ok(())
    }
}

/// `true` if this trcode is a `G7` on a 일반채권·국고채권 channel.
pub const fn handles(trcode: TrCode) -> bool {
    matches!(trcode.data_class(), [b'G', b'7']) && bond::is_bond_group(trcode)
}
