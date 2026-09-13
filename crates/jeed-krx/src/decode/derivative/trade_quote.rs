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

use crate::decode::derivative::{LEVEL_LEN, fill_book, fill_record_header, header, slice};
use crate::error::KrxError;
use crate::field;
use crate::trcode::TrCode;
use jeed_wire::{
    QuotePayload, RecordHeader, TradePayload, TradeQuotePayload, UnixNano, Venue, WireKind,
    WireRecord, trade_kind,
};

// Offsets of the trade block, from documents/krx/layouts.md.
const OFF_PRICE: usize = 47;
const OFF_QTY: usize = 56;
const OFF_CUMULATIVE_QTY: usize = 119;
const OFF_AGGRESSOR: usize = 153;
const OFF_DYN_UPPER: usize = 154;
const OFF_DYN_LOWER: usize = 163;

/// Bytes before the first book level — header plus the trade block.
pub const TRADE_BLOCK_END: usize = 172;

/// Bytes after the last level block.
const TAIL_LEN: usize = 29;

/// `IFMSRPD0037` — five levels per side.
pub const FIVE_DEEP: DerivativeTradeQuote = DerivativeTradeQuote::new(5);

/// `IFMSRPD0038` — ten levels per side (single-stock futures and options).
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

        let mut quote = QuotePayload::default();
        let shape = fill_book(payload, TRADE_BLOCK_END, self.depth, &mut quote)?;

        let price = field::decimal(slice(payload, OFF_PRICE, 9), OFF_PRICE)?;
        let qty = field::uint(slice(payload, OFF_QTY, 9), OFF_QTY)?;
        let cumulative = field::uint(slice(payload, OFF_CUMULATIVE_QTY, 12), OFF_CUMULATIVE_QTY)?;
        let dyn_upper = field::decimal(slice(payload, OFF_DYN_UPPER, 9), OFF_DYN_UPPER)?;
        let dyn_lower = field::decimal(slice(payload, OFF_DYN_LOWER, 9), OFF_DYN_LOWER)?;

        let mut trade =
            TradePayload::new(price.map_or(0, |d| d.value), qty.unwrap_or(0));

        if let Some(c) = cumulative {
            trade.with_cumulative_qty(c);
        }

        // 매도매수구분코드: space 단일가체결 / '0' 해당없음 / '1' 매도 / '2' 매수.
        // The first two are both "there was no aggressor" — an auction cross
        // has none by definition — so they collapse to UNKNOWN. That is
        // distinct from NONE, which means the channel has no such field at all.
        trade.with_kind(match payload[OFF_AGGRESSOR] {
            b'1' => trade_kind::SELL,
            b'2' => trade_kind::BUY,
            _ => trade_kind::UNKNOWN,
        });

        // KRX cannot say "not applicable": instruments outside the dynamic
        // limit regime (far-month futures, spreads) carry `000000.00` rather
        // than blanks. A real band always has both sides above zero, so that is
        // the test — and the answer rides in a flag so the consumer never has
        // to make it again.
        if let (Some(u), Some(l)) = (dyn_upper, dyn_lower)
            && u.value > 0
            && l.value > 0
        {
            trade.with_dyn_limits(u.value, l.value);
        }

        let mut h = RecordHeader::new(WireKind::TradeQuote, Venue::Krx, msg.isin, recv_ns);
        fill_record_header(&mut h, &msg, recv_ns, shape.price_scale);
        h.set_depth(shape.depth).set_flags(shape.flags);

        *out = WireRecord::new_trade_quote(h, TradeQuotePayload { trade, quote });
        Ok(())
    }
}

/// Book depth for a `G7` trcode, or `None` if it is not one.
pub const fn depth_for(trcode: TrCode) -> Option<usize> {
    if !trcode.is_derivative() {
        return None;
    }
    match trcode.data_class() {
        [b'G', b'7'] => match trcode.product_group() {
            [b'0', b'4', b'F'] | [b'0', b'5', b'F'] => Some(10),
            _ => Some(5),
        },
        _ => None,
    }
}
