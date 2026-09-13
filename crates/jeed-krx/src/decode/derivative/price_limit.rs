//! `V1` — 파생 가격제한폭확대발동. `IFMSRPD0043`, 65 B.
//!
//! The **daily** limit band, which widens in discrete stages when the market
//! presses against it. Not to be confused with the intraday dynamic band that
//! moves with every print — that is `Q2`
//! ([`dynamic_limit`](super::dynamic_limit)), and an order has to clear both
//! (`documents/krx/실시간가격제한.md`).
//!
//! ```text
//! [0:33]   header (shape C — no 세션ID)
//! [33:42]  가격확대시각          HHMMSSmmm, milliseconds, not micro
//! [42:44]  가격제한확대상한단계  → upper_stage
//! [44:46]  가격제한확대하한단계  → lower_stage
//! [46:55]  상한가                → upper_price
//! [55:64]  하한가                → lower_price
//! [64:65]  0xFF
//! ```
//!
//! The two stages move **independently** — the band can widen upward without
//! widening downward — so they are two fields, not one level.
//!
//! ## This is the channel that blanks its sequence number
//!
//! `V103F` still sends 정보분배일련번호 as spaces. That is why the wire's
//! `sequence` is documented as "may be zero because the channel left it blank"
//! rather than as a gap detector: ordering authority is `producer_seq`
//! (`CLAUDE.md`).

use crate::decode::common::slice;
use crate::decode::derivative::{LIMIT_HEADER_LEN, fill_record_header, limit_header};
use crate::error::KrxError;
use crate::extract;
use crate::field;
use crate::trcode::TrCode;
use jeed_wire::{PriceLimitPayload, RecordHeader, UnixNano, Venue, WireKind, WireRecord};

// Offsets from documents/krx/layouts.md.
const OFF_TIME: usize = 33;
const TIME_LEN: usize = 9;
const OFF_UPPER_STAGE: usize = 42;
const OFF_LOWER_STAGE: usize = 44;
const OFF_UPPER_PRICE: usize = 46;
const OFF_LOWER_PRICE: usize = 55;

/// `IFMSRPD0043`.
pub const MESSAGE_LEN: usize = 65;

const _: () = assert!(LIMIT_HEADER_LEN == OFF_TIME);

/// The `V1` 가격제한폭확대발동 decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DerivativePriceLimit;

/// `IFMSRPD0043`.
pub const DECODER: DerivativePriceLimit = DerivativePriceLimit;

impl DerivativePriceLimit {
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
        let msg = limit_header(payload, OFF_TIME, TIME_LEN)?;

        let price = extract::derivative_price(msg.trcode, &msg.isin);
        let upper = field::price(price, slice(payload, OFF_UPPER_PRICE, 9), OFF_UPPER_PRICE)?;
        let lower = field::price(price, slice(payload, OFF_LOWER_PRICE, 9), OFF_LOWER_PRICE)?;
        let upper_stage = field::uint(slice(payload, OFF_UPPER_STAGE, 2), OFF_UPPER_STAGE)?;
        let lower_stage = field::uint(slice(payload, OFF_LOWER_STAGE, 2), OFF_LOWER_STAGE)?;

        let payload_out = PriceLimitPayload {
            // 가격확대시각 is dateless like every other KRX clock field; the
            // absolute stamp goes on the record header, and this stays as the
            // exchange spelled it.
            applied_time_of_day: msg.time_of_day_ns.unwrap_or(0),
            upper_price: upper.unwrap_or(0),
            lower_price: lower.unwrap_or(0),
            // Blank stays zero here, as the field's own documentation says: the
            // channel blanks it and there is nothing to reconstruct.
            sequence: msg.sequence.unwrap_or(0) as u32,
            board_id: msg.board,
            information_category: msg.trcode.product_group(),
            upper_stage: upper_stage.unwrap_or(0) as u8,
            lower_stage: lower_stage.unwrap_or(0) as u8,
            _pad: [0; 5],
        };

        let mut h = RecordHeader::new(WireKind::PriceLimit, Venue::Krx, crate::field::wire_symbol(&msg.isin), recv_ns);
        fill_record_header(&mut h, &msg, recv_ns, price.scale().unwrap_or_default());

        *out = WireRecord::new_price_limit(h, payload_out);
        Ok(())
    }
}

/// `true` if this trcode is a `V1` on a derivative channel.
pub const fn handles(trcode: TrCode) -> bool {
    trcode.is_derivative() && matches!(trcode.data_class(), [b'V', b'1'])
}
