//! `B6` — 주식 우선호가 (MM/LP호가 제외). `IFMSRPD0002`, 590 B.
//!
//! ```text
//! [0:47]     header
//! [47:507]   10 × 46 B level block — 매도가11 매수가11 매도잔량12 매수잔량12
//! [507:519]  매도호가공개단계잔량합계  ← no wire slot
//! [519:531]  매수호가공개단계잔량합계  ← no wire slot
//! [531:542]  예상체결가                → quote_ext
//! [542:554]  예상체결수량              ← no wire slot
//! [554:565]  중간가격                  ← no wire slot
//! [565:577]  매도중간가호가총잔량      ← no wire slot
//! [577:589]  매수중간가호가총잔량      ← no wire slot
//! [589:590]  0xFF
//! ```
//!
//! Ten levels, always — 주식 has no five-deep variant the way the derivative
//! channels do, so there is nothing to select and no `depth_for` here.

use crate::decode::common::{fill_record_header, header};
use crate::decode::securities::{self, DEPTH, PLAIN};
use crate::error::KrxError;
use crate::extract::KRX;
use crate::trcode::TrCode;
use jeed_wire::{QuotePayload, RecordHeader, UnixNano, Venue, WireKind, WireRecord};

/// Bytes before the first level — the shape-A header.
const FIRST_LEVEL: usize = crate::decode::common::HEADER_LEN;

/// `IFMSRPD0002`.
pub const MESSAGE_LEN: usize = FIRST_LEVEL + DEPTH * PLAIN.stride + securities::TAIL_LEN;

const _: () = assert!(MESSAGE_LEN == 590);

/// The 주식 우선호가 decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StockQuote;

/// `IFMSRPD0002`.
pub const DECODER: StockQuote = StockQuote;

impl StockQuote {
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
        let msg = header(payload)?;

        let mut quote = QuotePayload::default();
        let shape = securities::fill_book(payload, FIRST_LEVEL, PLAIN, &mut quote)?;
        securities::fill_expected_price(payload, FIRST_LEVEL + DEPTH * PLAIN.stride, &mut quote)?;

        let mut h = RecordHeader::new(WireKind::Quote, Venue::Krx, crate::field::wire_symbol(&msg.isin), recv_ns);
        fill_record_header(&mut h, &msg, recv_ns, KRX.securities_price.scale().unwrap_or_default());
        h.set_depth(shape.depth).set_flags(shape.flags);

        *out = WireRecord::new_quote(h, quote);
        Ok(())
    }
}

/// `true` if this trcode is a `B6` on a 주식 channel.
pub const fn handles(trcode: TrCode) -> bool {
    matches!(trcode.data_class(), [b'B', b'6']) && super::is_stock_group(trcode)
}
