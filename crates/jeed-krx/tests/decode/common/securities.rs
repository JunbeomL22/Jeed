//! 증권 message builders — 주식 `B6`, ETF·ELW·ETN `B7`, 공통 체결 `A3`.

use super::{Header, num, put};

/// One 증권 book level. `lp` is written only by the `B7` builder.
#[derive(Debug, Clone, Copy)]
pub struct EquityLevel {
    /// 매도호가가격, exactly eleven bytes (`"00000074100"`).
    pub ask_price: &'static str,
    /// 매수호가가격, eleven bytes.
    pub bid_price: &'static str,
    /// 매도호가잔량.
    pub ask_qty: u64,
    /// 매수호가잔량.
    pub bid_qty: u64,
    /// 매도LP호가잔량 — the LP's share of `ask_qty`, not extra to it.
    pub ask_lp: u64,
    /// 매수LP호가잔량.
    pub bid_lp: u64,
}

impl EquityLevel {
    /// An empty level: zeros, not blanks.
    pub const EMPTY: Self = Self {
        ask_price: "00000000000",
        bid_price: "00000000000",
        ask_qty: 0,
        bid_qty: 0,
        ask_lp: 0,
        bid_lp: 0,
    };

    fn write(&self, buf: &mut Vec<u8>, lp: bool) {
        put(buf, 11, self.ask_price);
        put(buf, 11, self.bid_price);
        num(buf, 12, self.ask_qty);
        num(buf, 12, self.bid_qty);
        if lp {
            num(buf, 12, self.ask_lp);
            num(buf, 12, self.bid_lp);
        }
    }
}

/// A ten-deep 삼성전자 book around 74,100원.
pub fn samsung_book() -> Vec<EquityLevel> {
    (0..10)
        .map(|i| {
            let step = i as u64 * 100;
            EquityLevel {
                ask_price: PRICES_ASK[i],
                bid_price: PRICES_BID[i],
                ask_qty: 100 + step,
                bid_qty: 200 + step,
                ask_lp: 10 + i as u64,
                bid_lp: 20 + i as u64,
            }
        })
        .collect()
}

const PRICES_ASK: [&str; 10] = [
    "00000074100",
    "00000074200",
    "00000074300",
    "00000074400",
    "00000074500",
    "00000074600",
    "00000074700",
    "00000074800",
    "00000074900",
    "00000075000",
];
const PRICES_BID: [&str; 10] = [
    "00000074000",
    "00000073900",
    "00000073800",
    "00000073700",
    "00000073600",
    "00000073500",
    "00000073400",
    "00000073300",
    "00000073200",
    "00000073100",
];

/// The tail both 증권 우선호가 interfaces end with: 총잔량 ×2, 예상체결가,
/// 예상체결수량, 중간가격, 중간가호가총잔량 ×2, `0xFF`.
#[derive(Debug, Clone)]
pub struct EquityQuoteTail {
    /// 매도/매수호가공개단계잔량합계.
    pub totals: (u64, u64),
    /// 예상체결가, eleven bytes.
    pub expected_price: &'static str,
    /// 예상체결수량.
    pub expected_qty: u64,
    /// 중간가격, eleven bytes.
    pub mid_price: &'static str,
    /// 매도/매수중간가호가총잔량.
    pub mid_totals: (u64, u64),
}

impl Default for EquityQuoteTail {
    fn default() -> Self {
        Self {
            totals: (0, 0),
            expected_price: "00000000000",
            expected_qty: 0,
            mid_price: "00000000000",
            mid_totals: (0, 0),
        }
    }
}

impl EquityQuoteTail {
    fn write(&self, buf: &mut Vec<u8>) {
        num(buf, 12, self.totals.0);
        num(buf, 12, self.totals.1);
        put(buf, 11, self.expected_price);
        num(buf, 12, self.expected_qty);
        put(buf, 11, self.mid_price);
        num(buf, 12, self.mid_totals.0);
        num(buf, 12, self.mid_totals.1);
        buf.push(0xFF);
    }
}

/// A `B6` 주식 우선호가 message (`IFMSRPD0002`).
#[derive(Debug, Clone)]
pub struct StockB6 {
    /// The 47-byte shape-A header.
    pub header: Header,
    /// Ten levels, shallowest first.
    pub levels: Vec<EquityLevel>,
    /// The shared tail.
    pub tail: EquityQuoteTail,
}

impl StockB6 {
    /// A 삼성전자 book on the 유가증권 board.
    pub fn samsung() -> Self {
        Self {
            header: Header {
                trcode: "B601S",
                isin: "KR7005930003",
                ..Header::KOSPI200_FUTURES
            },
            levels: samsung_book(),
            tail: EquityQuoteTail::default(),
        }
    }

    /// Serialises to bytes, checking the total against the interface length.
    pub fn build(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.header.write(&mut buf);
        for l in &self.levels {
            l.write(&mut buf, false);
        }
        self.tail.write(&mut buf);
        assert_eq!(buf.len(), 590, "B6 주식 is IFMSRPD0002");
        buf
    }
}

/// A `B7` ETF·ELW·ETN 우선호가 message (`IFMSRPD0003`).
#[derive(Debug, Clone)]
pub struct EtfB7 {
    /// The 47-byte shape-A header.
    pub header: Header,
    /// Ten levels, shallowest first.
    pub levels: Vec<EquityLevel>,
    /// The shared tail.
    pub tail: EquityQuoteTail,
}

impl EtfB7 {
    /// A KODEX 200 book.
    pub fn kodex200() -> Self {
        Self {
            header: Header {
                trcode: "B703S",
                isin: "KR7069500007",
                ..Header::KOSPI200_FUTURES
            },
            levels: samsung_book(),
            tail: EquityQuoteTail::default(),
        }
    }

    /// Serialises to bytes, checking the total against the interface length.
    pub fn build(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.header.write(&mut buf);
        for l in &self.levels {
            l.write(&mut buf, true);
        }
        self.tail.write(&mut buf);
        assert_eq!(buf.len(), 830, "B7 is IFMSRPD0003");
        buf
    }
}

/// An `A3` 증권 체결 message (`IFMSRPD0004`) — 주식과 ETF 공통.
#[derive(Debug, Clone)]
pub struct EquityA3 {
    /// The 47-byte shape-A header.
    pub header: Header,
    /// 전일대비구분코드.
    pub prev_sign: u8,
    /// 전일대비가격, eleven bytes.
    pub prev_change: &'static str,
    /// 체결가격, eleven bytes.
    pub price: &'static str,
    /// 거래량.
    pub qty: u64,
    /// 시가 / 고가 / 저가, eleven bytes each.
    pub session_prices: (&'static str, &'static str, &'static str),
    /// 누적거래량.
    pub cumulative_qty: u64,
    /// 누적거래대금, twenty-two bytes.
    pub cumulative_value: &'static str,
    /// 최종매도매수구분코드.
    pub aggressor: u8,
    /// LP보유수량, fifteen bytes (may be negative for ETN).
    pub lp_holdings: &'static str,
    /// 매도/매수최우선호가가격, eleven bytes each.
    pub bbo: (&'static str, &'static str),
}

impl EquityA3 {
    /// A 삼성전자 print at 74,100원.
    pub fn samsung() -> Self {
        Self {
            header: Header {
                trcode: "A301S",
                isin: "KR7005930003",
                ..Header::KOSPI200_FUTURES
            },
            prev_sign: b'2',
            prev_change: "00000000100",
            price: "00000074100",
            qty: 37,
            session_prices: ("00000073900", "00000074500", "00000073700"),
            cumulative_qty: 4_812_355,
            cumulative_value: "0000000356472890000.00",
            aggressor: b'2',
            lp_holdings: "000000000000000",
            bbo: ("00000074100", "00000074000"),
        }
    }

    /// Serialises to bytes, checking the total against the interface length.
    pub fn build(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.header.write(&mut buf);
        buf.push(self.prev_sign);
        put(&mut buf, 11, self.prev_change);
        put(&mut buf, 11, self.price);
        num(&mut buf, 10, self.qty);
        put(&mut buf, 11, self.session_prices.0);
        put(&mut buf, 11, self.session_prices.1);
        put(&mut buf, 11, self.session_prices.2);
        num(&mut buf, 12, self.cumulative_qty);
        put(&mut buf, 22, self.cumulative_value);
        buf.push(self.aggressor);
        put(&mut buf, 15, self.lp_holdings);
        put(&mut buf, 11, self.bbo.0);
        put(&mut buf, 11, self.bbo.1);
        buf.push(0xFF);
        assert_eq!(buf.len(), 186, "A3 증권 is IFMSRPD0004");
        buf
    }
}
