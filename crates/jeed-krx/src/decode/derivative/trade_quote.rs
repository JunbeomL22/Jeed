//! `G7` — 파생 체결 + 우선호가. `IFMSRPD0037` (5-deep, 431 B) and
//! `IFMSRPD0038` (10-deep, 661 B).
//!
//! This is the channel Jeed is built around: the print and the book it left
//! behind arrive in **one** message, so they cannot be separated by loss or
//! reordering the way a `B6`/`A3` pair can. It also carries the dynamic price
//! limits inline, which is the only way to learn the current band without
//! having watched every `Q2` event since the session opened.
//!
//! ```text
//! [0:47]     header
//! [47:56]    체결가격
//! [56:65]    거래량
//! [65:74]    근월물체결가격        ← no wire slot (todo.md §10)
//! [74:83]    원월물체결가격        ← no wire slot
//! [83:92]    시가                  ← no wire slot (rebuildable from the tape)
//! [92:101]   고가                  ← no wire slot
//! [101:110]  저가                  ← no wire slot
//! [110:119]  직전가격              ← no wire slot
//! [119:131]  누적거래량            → TradePayload::cumulative_qty
//! [131:153]  누적거래대금          ← no wire slot
//! [153:154]  최종매도매수구분코드  → TradePayload::trade_kind
//! [154:163]  동적상한가            → TradePayload::dyn_upper
//! [163:172]  동적하한가            → TradePayload::dyn_lower
//! [172:..]   depth × 46 B level block
//! [..+0:18]  호가총잔량 ×2         ← no wire slot
//! [..+18:28] 호가유효건수 ×2       ← no wire slot
//! [..+28:29] 0xFF
//! ```
//!
//! > The dynamic-limit offsets above are `[154:163]`/`[163:172]`. Reading the
//! > spec's offset column as a *start* gives `[163:172]`/`[172:181]`, which
//! > parses without complaint and yields an upper limit **below** the lower
//! > one. That is the whole reason `documents/krx/layouts.md` is generated.

use crate::decode::derivative::{
    LEVEL_LEN, TRADE_BLOCK_END, fill_book, fill_record_header, fill_trade, header,
};
use crate::error::KrxError;
use crate::extract;
use crate::trcode::TrCode;
use jeed_wire::{
    QuotePayload, RecordHeader, TradeQuotePayload, UnixNano, Venue, WireKind, WireRecord,
};

/// Bytes after the last level block.
const TAIL_LEN: usize = 29;

/// `IFMSRPD0037` — five levels per side.
pub const FIVE_DEEP: DerivativeTradeQuote = DerivativeTradeQuote::new(5);

/// `IFMSRPD0038` — ten levels per side (single-stock options `G705F` /
/// `G718F`; **not** single-stock futures — see [`depth_for`]).
pub const TEN_DEEP: DerivativeTradeQuote = DerivativeTradeQuote::new(10);

/// A `G7` decoder for one book depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DerivativeTradeQuote {
    depth: usize,
    message_len: usize,
}

impl DerivativeTradeQuote {
    /// Decoder for `depth` levels per side.
    pub const fn new(depth: usize) -> Self {
        Self { depth, message_len: TRADE_BLOCK_END + depth * LEVEL_LEN + TAIL_LEN }
    }

    /// Levels per side this variant carries.
    #[inline]
    pub const fn depth(&self) -> usize {
        self.depth
    }

    /// Fixed message length this variant expects.
    #[inline]
    pub const fn message_len(&self) -> usize {
        self.message_len
    }

    /// Decodes into `out`, leaving it untouched on error.
    pub fn decode(
        &self,
        payload: &[u8],
        recv_ns: UnixNano,
        out: &mut WireRecord,
    ) -> Result<(), KrxError> {
        crate::message::validate(payload, self.message_len)?;
        let msg = header(payload)?;

        let price = extract::derivative_price(msg.trcode, &msg.isin);
        let price_scale = price.scale().unwrap_or_default();

        let mut quote = QuotePayload::default();
        let shape = fill_book(payload, TRADE_BLOCK_END, self.depth, price, &mut quote)?;
        let trade = fill_trade(payload, price)?;

        let mut h = RecordHeader::new(WireKind::TradeQuote, Venue::Krx, msg.isin, recv_ns);
        fill_record_header(&mut h, &msg, recv_ns, price_scale);
        h.set_depth(shape.depth).set_flags(shape.flags);

        *out = WireRecord::new_trade_quote(h, TradeQuotePayload { trade, quote });
        Ok(())
    }
}

/// Book depth for a `G7` trcode, or `None` if it is not one.
///
/// Same rule as [`quote::depth_for`](super::quote::depth_for), including the
/// single-stock futures trap: `G704F` is five-deep (431 B), not ten.
pub const fn depth_for(trcode: TrCode) -> Option<usize> {
    if !trcode.is_derivative() {
        return None;
    }
    match trcode.data_class() {
        [b'G', b'7'] => Some(super::depth_for_product_group(trcode)),
        _ => None,
    }
}
