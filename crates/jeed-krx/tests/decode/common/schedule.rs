//! `M4` 장운영스케줄공개 builder — 시장 공통 전문 하나.

use super::{num, put};

/// An `M4` 장운영스케줄공개 message (`IFMSRPD0019`), 83 B.
///
/// This one opens with no real-time header at all: no 정보분배일련번호, no
/// 정보분배종목인덱스, and the 종목코드 sits 31 bytes in. It is a control
/// message, not a tick.
#[derive(Debug, Clone)]
pub struct M4 {
    /// 데이터구분값 + 정보구분값, five bytes.
    pub trcode: &'static str,
    /// 장운영상품그룹ID, three bytes.
    pub product_group_id: &'static str,
    /// 보드ID.
    pub board: &'static str,
    /// 보드이벤트ID, three bytes.
    pub event_id: &'static str,
    /// 보드이벤트시작시각 `HHMMSSmmm` — milliseconds, like `V1`.
    pub event_time: &'static str,
    /// 보드이벤트적용군코드.
    pub event_group: u64,
    /// 세션개시종료코드, two bytes.
    pub session_action: &'static str,
    /// 세션ID.
    pub session: &'static str,
    /// 종목코드, twelve bytes — **spaces** for a product- or board-wide notice.
    pub isin: &'static str,
    /// 상장사종목코드, twelve bytes (파생 미해당).
    pub common_stock_isin: &'static str,
    /// 상품ID, eleven bytes.
    pub product_id: &'static str,
    /// 거래정지사유코드, three bytes (파생 미해당).
    pub halt_reason: &'static str,
    /// 거래정지발생유형코드 (파생 미해당).
    pub halt_type: u8,
    /// 적용단계.
    pub step: u64,
    /// 기준종목가격제한확대발생코드 (파생 해당).
    pub expansion: u8,
    /// 가격제한확대예정시각 `HHMMSSmmm` (파생 해당).
    pub expected_time: &'static str,
}

impl M4 {
    /// A derivative session-start notice, scoped to a product rather than an
    /// instrument — so the 종목코드 is twelve spaces.
    pub fn derivative_session_start() -> Self {
        Self {
            trcode: "M401F",
            product_group_id: "101",
            board: "G1",
            event_id: "BS1",
            event_time: "090000000",
            event_group: 0,
            session_action: "BS",
            session: "40",
            isin: "            ",
            common_stock_isin: "            ",
            product_id: "KR4101V9000",
            halt_reason: "   ",
            halt_type: b' ',
            step: 0,
            expansion: b' ',
            expected_time: "         ",
        }
    }

    /// A notice announcing that one instrument's daily band is about to widen.
    pub fn expansion_notice() -> Self {
        Self {
            event_id: "PE1",
            isin: "KR4101V90009",
            step: 1,
            expansion: b'1',
            expected_time: "101500000",
            ..Self::derivative_session_start()
        }
    }

    /// Serialises to bytes, checking the total against the interface length.
    pub fn build(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        put(&mut buf, 5, self.trcode);
        put(&mut buf, 3, self.product_group_id);
        put(&mut buf, 2, self.board);
        put(&mut buf, 3, self.event_id);
        put(&mut buf, 9, self.event_time);
        num(&mut buf, 5, self.event_group);
        put(&mut buf, 2, self.session_action);
        put(&mut buf, 2, self.session);
        put(&mut buf, 12, self.isin);
        put(&mut buf, 12, self.common_stock_isin);
        put(&mut buf, 11, self.product_id);
        put(&mut buf, 3, self.halt_reason);
        buf.push(self.halt_type);
        num(&mut buf, 2, self.step);
        buf.push(self.expansion);
        put(&mut buf, 9, self.expected_time);
        buf.push(0xFF);
        assert_eq!(buf.len(), 83, "M4 is IFMSRPD0019");
        buf
    }
}
