//! `B7` — ETF·ELW·ETN 우선호가 (MM/LP호가 포함). `IFMSRPD0003`, 830 B.
//!
//! ```text
//! [0:47]     header
//! [47:747]   10 × 70 B level block — 매도가11 매수가11 매도잔량12 매수잔량12
//!                                    매도LP잔량12 매수LP잔량12
//! [747:759]  매도호가공개단계잔량합계  ← no wire slot
//! [759:771]  매수호가공개단계잔량합계  ← no wire slot
//! [771:782]  예상체결가                → quote_ext
//! [782:794]  예상체결수량              ← no wire slot
//! [794:805]  중간가격                  ← no wire slot
//! [805:817]  매도중간가호가총잔량      ← no wire slot
//! [817:829]  매수중간가호가총잔량      ← no wire slot
//! [829:830]  0xFF
//! ```
//!
//! ## The LP quantity is part of the level, not extra to it
//!
//! 매도1단계LP우선호가잔량 is the liquidity provider's share of the quantity
//! already reported at that level, not additional size behind it. So it rides
//! in the level's `ext` word ([`level_ext::LP_QUANTITY`](jeed_wire::level_ext::LP_QUANTITY))
//! and the level's `qty` stays the whole resting amount — a consumer that
//! ignores `ext` still sees a correct book.
//!
//! It is worth having because LP size behaves differently from everyone else's:
//! it is quoted to an obligation rather than to a view, and it is the part of
//! the level most likely to still be there.

use crate::decode::common::{fill_record_header, header};
use crate::decode::securities::{self, DEPTH, WITH_LP};
use crate::error::KrxError;
use crate::extract::KRX;
use crate::trcode::TrCode;
use jeed_wire::{QuotePayload, RecordHeader, UnixNano, Venue, WireKind, WireRecord};

/// Bytes before the first level — the shape-A header.
const FIRST_LEVEL: usize = crate::decode::common::HEADER_LEN;

/// `IFMSRPD0003`.
pub const MESSAGE_LEN: usize = FIRST_LEVEL + DEPTH * WITH_LP.stride + securities::TAIL_LEN;

const _: () = assert!(MESSAGE_LEN == 830);

/// The ETF·ELW·ETN 우선호가 decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EtfQuote;

/// `IFMSRPD0003`.
pub const DECODER: EtfQuote = EtfQuote;

impl EtfQuote {
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
        let shape = securities::fill_book(payload, FIRST_LEVEL, WITH_LP, &mut quote)?;
        securities::fill_expected_price(payload, FIRST_LEVEL + DEPTH * WITH_LP.stride, &mut quote)?;

        let mut h = RecordHeader::new(WireKind::Quote, Venue::Krx, crate::field::wire_symbol(&msg.isin), recv_ns);
        fill_record_header(&mut h, &msg, recv_ns, KRX.securities_price.scale().unwrap_or_default());
        h.set_depth(shape.depth).set_flags(shape.flags);

        *out = WireRecord::new_quote(h, quote);
        Ok(())
    }
}

/// `true` if this trcode is a `B7`.
///
/// There is no other market on this data class — `B7` is 증권 LP호가 and
/// nothing else — but the product group is checked anyway so that a code from
/// a market this build does not decode cannot land here.
pub const fn handles(trcode: TrCode) -> bool {
    matches!(trcode.data_class(), [b'B', b'7']) && super::is_lp_group(trcode)
}
