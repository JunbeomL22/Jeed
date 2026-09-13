//! `Q2` — 파생 동적상하한가 적용 및 해제. `IFMSRPD0042`, 65 B.
//!
//! ```text
//! [0:33]   header (shape C — no 세션ID)
//! [33:45]  매매처리시각          HHMMSSuuuuuu, microseconds
//! [45:46]  동적가격제한설정코드  → action
//! [46:55]  동적상한가            → upper_price
//! [55:64]  동적하한가            → lower_price
//! [64:65]  0xFF
//! ```
//!
//! ## Why this exists when `G7` already carries the band
//!
//! `G7` and `A3` carry the band in force *after a print*. That covers the band
//! moving, and it is the cheaper path — no extra message to wait for. It does
//! not cover the band being **released**: a release comes with no trade, so a
//! consumer watching only prints keeps believing the last band it saw, and goes
//! on rejecting its own orders against a fence the exchange has taken down.
//!
//! It is a rare message — 1,439 in a day against 66.9M derivative messages —
//! which is exactly why it is easy to leave out and expensive to have left out.
//!
//! ## The regime does not cover everything
//!
//! 원월물, 선물스프레드, 주식옵션, 돈육선물 and other thin instruments are
//! outside it entirely (`documents/krx/실시간가격제한.md`). Those never send
//! this message; they carry `000000.00` in the `G7` band fields instead, which
//! is why that path needs a validity flag and this one does not.

use crate::decode::common::slice;
use crate::decode::derivative::{LIMIT_HEADER_LEN, fill_record_header, limit_header};
use crate::error::KrxError;
use crate::extract;
use crate::field;
use crate::trcode::TrCode;
use jeed_wire::{
    DynamicPriceLimitPayload, RecordHeader, UnixNano, Venue, WireKind, WireRecord,
    dyn_limit_action,
};

// Offsets from documents/krx/layouts.md.
const OFF_TIME: usize = 33;
const TIME_LEN: usize = 12;
const OFF_ACTION: usize = 45;
const OFF_UPPER_PRICE: usize = 46;
const OFF_LOWER_PRICE: usize = 55;

/// `IFMSRPD0042`.
pub const MESSAGE_LEN: usize = 65;

const _: () = assert!(LIMIT_HEADER_LEN == OFF_TIME);

/// The `Q2` 동적상하한가 적용·해제 decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DerivativeDynamicLimit;

/// `IFMSRPD0042`.
pub const DECODER: DerivativeDynamicLimit = DerivativeDynamicLimit;

impl DerivativeDynamicLimit {
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

        // 동적가격제한설정코드: '1' 적용, '2' 해제. An unrecognised byte becomes
        // UNKNOWN rather than a guess, and UNKNOWN reads the same as RELEASED
        // downstream — do not fence orders with these numbers — which is the
        // safe direction to be wrong in.
        let action = match payload[OFF_ACTION] {
            b'1' => dyn_limit_action::APPLIED,
            b'2' => dyn_limit_action::RELEASED,
            _ => dyn_limit_action::UNKNOWN,
        };

        let out_payload = DynamicPriceLimitPayload {
            applied_time_of_day: msg.time_of_day_ns.unwrap_or(0),
            upper_price: upper.unwrap_or(0),
            lower_price: lower.unwrap_or(0),
            sequence: msg.sequence.unwrap_or(0) as u32,
            board_id: msg.board,
            information_category: msg.trcode.product_group(),
            action,
            _pad: [0; 6],
        };

        let mut h = RecordHeader::new(WireKind::DynamicPriceLimit, Venue::Krx, crate::field::wire_symbol(&msg.isin), recv_ns);
        fill_record_header(&mut h, &msg, recv_ns, price.scale().unwrap_or_default());

        *out = WireRecord::new_dynamic_price_limit(h, out_payload);
        Ok(())
    }
}

/// `true` if this trcode is a `Q2` on a derivative channel.
pub const fn handles(trcode: TrCode) -> bool {
    trcode.is_derivative() && matches!(trcode.data_class(), [b'Q', b'2'])
}
