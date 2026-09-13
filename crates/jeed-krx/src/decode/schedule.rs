//! `M4` — 장운영스케줄공개. `IFMSRPD0019`, 83 B.
//!
//! **One interface for every market.** 파생·증권·채권·금현물·배출권 all send
//! the same 83 bytes, which is why this sits beside the market modules rather
//! than inside one: `M401F`, `M401S` and `M401B` differ only in the product
//! group, and the layout does not change with it.
//!
//! ```text
//! [0:5]    데이터구분 + 정보구분
//! [5:8]    장운영상품그룹ID        → market_operation_product_id
//! [8:10]   보드ID                  → board_id
//! [10:13]  보드이벤트ID            → board_event_id
//! [13:22]  보드이벤트시작시각      → event_time_of_day  (HHMMSSmmm)
//! [22:27]  보드이벤트적용군코드    → board_event_group
//! [27:29]  세션개시종료코드        → session_action
//! [29:31]  세션ID                  → session_id
//! [31:43]  종목코드                → header ISIN
//! [43:55]  상장사종목코드          → common_stock_isin   (파생 미해당)
//! [55:66]  상품ID                  → product_id
//! [66:69]  거래정지사유코드        → halt_reason         (파생 미해당)
//! [69:70]  거래정지발생유형코드    → halt_type           (파생 미해당)
//! [70:72]  적용단계                → step
//! [72:73]  기준종목가격제한확대발생코드 → expansion_direction (파생 해당)
//! [73:82]  가격제한확대예정시각    → expected_time_of_day (파생 해당)
//! [82:83]  0xFF
//! ```
//!
//! ## There is no header here
//!
//! This message does not open with any of the three real-time header shapes:
//! no 정보분배일련번호, no 정보분배종목인덱스, and the 종목코드 sits 31 bytes
//! in. It is a control message, not a tick.
//!
//! ## Scope is carried, not resolved
//!
//! A notice can apply to one instrument, to a product, or to a whole board, and
//! which it is depends on fields the consumer may weigh differently. So the
//! scope fields are passed through verbatim and only the one unambiguous fact —
//! whether the 종목코드 is a real instrument or twelve spaces — becomes a flag
//! ([`schedule_flags::INSTRUMENT_SCOPED`]).

use crate::decode::common::slice;
use crate::error::KrxError;
use crate::field;
use crate::trcode::TrCode;
use jeed_wire::{
    ISIN_LEN, MarketSchedulePayload, RecordHeader, UnixNano, Venue, WireKind, WireRecord,
    expansion_direction, schedule_flags,
};

// Offsets from documents/krx/layouts.md.
const OFF_PRODUCT_GROUP: usize = 5;
const OFF_BOARD: usize = 8;
const OFF_EVENT_ID: usize = 10;
const OFF_EVENT_TIME: usize = 13;
const OFF_EVENT_GROUP: usize = 22;
const OFF_SESSION_ACTION: usize = 27;
const OFF_SESSION_ID: usize = 29;
const OFF_ISIN: usize = 31;
const OFF_COMMON_STOCK_ISIN: usize = 43;
const OFF_PRODUCT_ID: usize = 55;
const OFF_HALT_REASON: usize = 66;
const OFF_HALT_TYPE: usize = 69;
const OFF_STEP: usize = 70;
const OFF_EXPANSION: usize = 72;
const OFF_EXPECTED_TIME: usize = 73;

/// `IFMSRPD0019`.
pub const MESSAGE_LEN: usize = 83;

/// The 장운영스케줄공개 decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MarketSchedule;

/// `IFMSRPD0019`.
pub const DECODER: MarketSchedule = MarketSchedule;

/// Copies a fixed-width byte field.
#[inline]
fn bytes<const N: usize>(payload: &[u8], at: usize) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(slice(payload, at, N));
    out
}

impl MarketSchedule {
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
        let trcode = TrCode::from_message(payload)?;

        let event_time = field::time_of_day_ns(slice(payload, OFF_EVENT_TIME, 9), OFF_EVENT_TIME)?;
        let expected_time =
            field::time_of_day_ns(slice(payload, OFF_EXPECTED_TIME, 9), OFF_EXPECTED_TIME)?;
        let event_group = field::uint(slice(payload, OFF_EVENT_GROUP, 5), OFF_EVENT_GROUP)?;
        let step = field::uint(slice(payload, OFF_STEP, 2), OFF_STEP)?;

        let isin: [u8; ISIN_LEN] = bytes(payload, OFF_ISIN);

        let mut flags = 0u8;
        if !field::is_blank(&isin) {
            flags |= schedule_flags::INSTRUMENT_SCOPED;
        }
        if expected_time.is_some() {
            flags |= schedule_flags::EXPECTED_TIME_VALID;
        }

        // 기준종목가격제한확대발생코드: '1' 상한 확대, '2' 하한 확대. Anything
        // else — including the space every non-derivative market sends — means
        // this notice is not announcing an expansion at all.
        let expansion = match payload[OFF_EXPANSION] {
            b'1' => expansion_direction::UP,
            b'2' => expansion_direction::DOWN,
            _ => expansion_direction::NOT_APPLICABLE,
        };

        let schedule = MarketSchedulePayload {
            event_time_of_day: event_time.unwrap_or(0),
            expected_time_of_day: expected_time.unwrap_or(0),
            board_event_group: event_group.unwrap_or(0) as u32,
            information_category: trcode.product_group(),
            market_operation_product_id: bytes(payload, OFF_PRODUCT_GROUP),
            board_id: bytes(payload, OFF_BOARD),
            board_event_id: bytes(payload, OFF_EVENT_ID),
            session_action: bytes(payload, OFF_SESSION_ACTION),
            session_id: bytes(payload, OFF_SESSION_ID),
            product_id: bytes(payload, OFF_PRODUCT_ID),
            common_stock_isin: bytes(payload, OFF_COMMON_STOCK_ISIN),
            halt_reason: bytes(payload, OFF_HALT_REASON),
            halt_type: payload[OFF_HALT_TYPE],
            step: step.unwrap_or(0) as u8,
            expansion_direction: expansion,
            schedule_flags: flags,
            _pad: [0; 7],
        };

        let mut h = RecordHeader::new(WireKind::MarketSchedule, Venue::Krx, isin, recv_ns);
        h.recv_ns = recv_ns;
        // 보드이벤트시작시각 is when the event starts, which is not necessarily
        // when this message was sent — it is a schedule, and the start can be in
        // the future. So it is not stamped as a venue timestamp; it stays in the
        // payload where it says what it is.

        *out = WireRecord::new_market_schedule(h, schedule);
        Ok(())
    }
}

/// `true` if this trcode is an `M4`.
///
/// Every market's, deliberately: one interface, one layout.
pub const fn handles(trcode: TrCode) -> bool {
    matches!(trcode.data_class(), [b'M', b'4'])
}
