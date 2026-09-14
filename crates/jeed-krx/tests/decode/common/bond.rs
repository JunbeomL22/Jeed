//! 채권 message builders — 우선호가 `B6`, 체결 `A3`, 체결+우선호가 `G7`, for
//! 일반채권·국고채권 (`IFMSRPD0023`/`0027`/`0029`) and 소액채권
//! (`IFMSRPD0024`/`0027`/`0030`).
//!
//! Prices are spelled the way the wire spells them, `[부호][정수 7][.][소수 2]`
//! — `00010345.50` is 10,345.50원. The first version of these builders wrote
//! eleven digits with no point, the decoder agreed, and every real bond
//! message was refused at byte 41 (`documents/todo.md` §16③). A builder that
//! matches the decoder's misreading proves nothing; the shape here is the
//! 참고-가격표시정보 sheet's.

use super::{blank, num, put};

/// The 41-byte shape-B header.
///
/// 채권 carries no 정보분배종목인덱스, so the clock sits six bytes earlier than
/// it does on every other real-time message. A test that built this with the
/// shape-A writer would still produce a parseable message — with every field
/// after the ISIN shifted — which is exactly the failure the length assertion
/// below is here to catch.
#[derive(Debug, Clone)]
pub struct BondHeader {
    /// 데이터구분값 + 정보구분값, five bytes.
    pub trcode: &'static str,
    /// 정보분배일련번호.
    pub sequence: Option<u64>,
    /// 보드ID.
    pub board: &'static str,
    /// 세션ID.
    pub session: &'static str,
    /// 종목코드, twelve bytes.
    pub isin: &'static str,
    /// 매매처리시각 `HHMMSSuuuuuu`.
    pub time: &'static str,
}

impl BondHeader {
    /// A 국고채권 header at 09:01:00.123456.
    pub const KTB: Self = Self {
        trcode: "B601K",
        sequence: Some(1),
        board: "G1",
        session: "40",
        isin: "KR103501GA98",
        time: "090100123456",
    };

    /// A 소액채권 (국민주택채권 1종) header at the same instant.
    pub const NHB: Self = Self {
        trcode: "B601M",
        sequence: Some(1),
        board: "G1",
        session: "40",
        isin: "KR2001022C74",
        time: "090100123456",
    };

    fn write(&self, buf: &mut Vec<u8>) {
        put(buf, 5, self.trcode);
        match self.sequence {
            Some(s) => num(buf, 8, s),
            None => blank(buf, 8),
        }
        put(buf, 2, self.board);
        put(buf, 2, self.session);
        put(buf, 12, self.isin);
        put(buf, 12, self.time);
        assert_eq!(buf.len(), 41, "bond header is 41 bytes");
    }
}

/// One 채권 book level: 가격 11 ×2, 잔량 15 ×2 (천원), 수익률 13 ×2.
#[derive(Debug, Clone, Copy)]
pub struct BondLevel {
    /// 매도호가가격, eleven bytes `[0][정수 7].[소수 2]`.
    pub ask_price: &'static str,
    /// 매수호가가격, eleven bytes `[0][정수 7].[소수 2]`.
    pub bid_price: &'static str,
    /// 채권매도호가잔량, 천원 단위.
    pub ask_qty: u64,
    /// 채권매수호가잔량, 천원 단위.
    pub bid_qty: u64,
    /// 매도호가수익률, thirteen bytes.
    pub ask_yield: &'static str,
    /// 매수호가수익률, thirteen bytes.
    pub bid_yield: &'static str,
}

impl BondLevel {
    /// An empty level: zeros, not blanks — and the point stays where it is.
    pub const EMPTY: Self = Self {
        ask_price: "00000000.00",
        bid_price: "00000000.00",
        ask_qty: 0,
        bid_qty: 0,
        ask_yield: "000000.000000",
        bid_yield: "000000.000000",
    };

    fn write(&self, buf: &mut Vec<u8>) {
        put(buf, 11, self.ask_price);
        put(buf, 11, self.bid_price);
        num(buf, 15, self.ask_qty);
        num(buf, 15, self.bid_qty);
        put(buf, 13, self.ask_yield);
        put(buf, 13, self.bid_yield);
    }
}

/// A three-deep 국고채 book around 10,345.50원 / 3.12%, padded to five.
///
/// Note the yields run the other way from the prices: a higher ask price is a
/// lower ask yield. A book built with both sorted the same way would pass every
/// structural check and still describe something that cannot happen.
pub fn ktb_book() -> Vec<BondLevel> {
    vec![
        BondLevel {
            ask_price: "00010345.50",
            bid_price: "00010340.00",
            ask_qty: 500_000,
            bid_qty: 300_000,
            ask_yield: "000003.123456",
            bid_yield: "000003.128900",
        },
        BondLevel {
            ask_price: "00010350.00",
            bid_price: "00010335.00",
            ask_qty: 700_000,
            bid_qty: 450_000,
            ask_yield: "000003.118000",
            bid_yield: "000003.134000",
        },
        BondLevel {
            ask_price: "00010355.00",
            bid_price: "00010330.00",
            ask_qty: 200_000,
            bid_qty: 150_000,
            ask_yield: "000003.112500",
            bid_yield: "000003.139500",
        },
        BondLevel::EMPTY,
        BondLevel::EMPTY,
    ]
}

/// The 채권 trade block `A3` and `G7` share, `[41:222]`.
#[derive(Debug, Clone)]
pub struct BondTradeBlock {
    /// 체결가격, eleven bytes.
    pub price: &'static str,
    /// 거래량, 천원 단위.
    pub qty: u64,
    /// 거래일자 `YYYYMMDD`.
    pub trade_date: &'static str,
    /// 거래대금, twenty-two bytes.
    pub value: &'static str,
    /// 체결수익률, thirteen bytes.
    pub trade_yield: &'static str,
    /// 시가 / 고가 / 저가, eleven bytes each.
    pub session_prices: (&'static str, &'static str, &'static str),
    /// 시가 / 고가 / 저가 수익률, thirteen bytes each.
    pub session_yields: (&'static str, &'static str, &'static str),
    /// 채권누적체결수량, 천원 단위.
    pub cumulative_qty: u64,
    /// 누적거래대금, twenty-two bytes.
    pub cumulative_value: &'static str,
    /// 결제일자 `YYYYMMDD`.
    pub settlement_date: &'static str,
}

impl Default for BondTradeBlock {
    fn default() -> Self {
        Self {
            price: "00010345.50",
            qty: 100_000,
            trade_date: "20260731",
            value: "000000001034550000.000",
            trade_yield: "000003.123456",
            session_prices: ("00010330.00", "00010355.00", "00010325.00"),
            session_yields: ("000003.139500", "000003.112500", "000003.145000"),
            cumulative_qty: 8_500_000,
            cumulative_value: "000000087932500000.000",
            settlement_date: "20260801",
        }
    }
}

impl BondTradeBlock {
    fn write(&self, buf: &mut Vec<u8>) {
        put(buf, 11, self.price);
        num(buf, 10, self.qty);
        put(buf, 8, self.trade_date);
        put(buf, 22, self.value);
        put(buf, 13, self.trade_yield);
        put(buf, 11, self.session_prices.0);
        put(buf, 11, self.session_prices.1);
        put(buf, 11, self.session_prices.2);
        put(buf, 13, self.session_yields.0);
        put(buf, 13, self.session_yields.1);
        put(buf, 13, self.session_yields.2);
        num(buf, 15, self.cumulative_qty);
        put(buf, 22, self.cumulative_value);
        put(buf, 8, self.settlement_date);
        assert_eq!(buf.len(), 222, "bond trade block ends at 222");
    }
}

/// A `B6` 채권 우선호가 message (`IFMSRPD0023`).
#[derive(Debug, Clone)]
pub struct BondB6 {
    /// The 41-byte header.
    pub header: BondHeader,
    /// Five levels, shallowest first.
    pub levels: Vec<BondLevel>,
    /// 채권매도/매수호가총잔량.
    pub totals: (u64, u64),
}

impl BondB6 {
    /// A 국고채 book.
    pub fn ktb(levels: Vec<BondLevel>) -> Self {
        Self { header: BondHeader::KTB, levels, totals: (1_400_000, 900_000) }
    }

    /// Serialises to bytes, checking the total against the interface length.
    pub fn build(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.header.write(&mut buf);
        for l in &self.levels {
            l.write(&mut buf);
        }
        num(&mut buf, 15, self.totals.0);
        num(&mut buf, 15, self.totals.1);
        buf.push(0xFF);
        assert_eq!(buf.len(), 462, "B6 채권 is IFMSRPD0023");
        buf
    }
}

/// An `A3` 채권 체결 message (`IFMSRPD0027`).
#[derive(Debug, Clone)]
pub struct BondA3 {
    /// The 41-byte header.
    pub header: BondHeader,
    /// The shared trade block.
    pub trade: BondTradeBlock,
}

impl BondA3 {
    /// A 국고채 print.
    pub fn ktb() -> Self {
        Self {
            header: BondHeader { trcode: "A301K", ..BondHeader::KTB },
            trade: BondTradeBlock::default(),
        }
    }

    /// A 소액채권 print — the same interface under `A301M`.
    pub fn nhb() -> Self {
        Self {
            header: BondHeader { trcode: "A301M", ..BondHeader::NHB },
            trade: BondTradeBlock::default(),
        }
    }

    /// Serialises to bytes, checking the total against the interface length.
    pub fn build(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.header.write(&mut buf);
        self.trade.write(&mut buf);
        buf.push(0xFF);
        assert_eq!(buf.len(), 223, "A3 채권 is IFMSRPD0027");
        buf
    }
}

/// A `G7` 채권 체결 + 우선호가 message (`IFMSRPD0029`).
#[derive(Debug, Clone)]
pub struct BondG7 {
    /// The 41-byte header.
    pub header: BondHeader,
    /// The shared trade block.
    pub trade: BondTradeBlock,
    /// Five levels, shallowest first.
    pub levels: Vec<BondLevel>,
    /// 채권매도/매수호가총잔량.
    pub totals: (u64, u64),
}

impl BondG7 {
    /// A 국고채 print with the book it left.
    pub fn ktb(levels: Vec<BondLevel>) -> Self {
        Self {
            header: BondHeader { trcode: "G701K", ..BondHeader::KTB },
            trade: BondTradeBlock::default(),
            levels,
            totals: (1_400_000, 900_000),
        }
    }

    /// Serialises to bytes, checking the total against the interface length.
    pub fn build(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.header.write(&mut buf);
        self.trade.write(&mut buf);
        for l in &self.levels {
            l.write(&mut buf);
        }
        num(&mut buf, 15, self.totals.0);
        num(&mut buf, 15, self.totals.1);
        buf.push(0xFF);
        assert_eq!(buf.len(), 643, "G7 채권 is IFMSRPD0029");
        buf
    }
}

// ===========================================================================
// 소액채권 — IFMSRPD0024 / IFMSRPD0030
// ===========================================================================

/// One 소액채권 book level: the instrument's own six fields, then the same
/// six again for the 채권종류 it belongs to. 156 bytes.
#[derive(Debug, Clone, Copy)]
pub struct SmallLotLevel {
    /// 종목 — the fields `IFMSRPD0023` carries at this level.
    pub own: BondLevel,
    /// 채권종류 — the exchange's aggregate across every issue of this kind
    /// and month. Not carried on the wire; here so a test can prove it is
    /// stepped over rather than read.
    pub class: BondLevel,
}

impl SmallLotLevel {
    /// Both blocks empty.
    pub const EMPTY: Self = Self { own: BondLevel::EMPTY, class: BondLevel::EMPTY };

    fn write(&self, buf: &mut Vec<u8>) {
        let start = buf.len();
        self.own.write(buf);
        self.class.write(buf);
        assert_eq!(buf.len() - start, 156, "small-lot level is 156 bytes");
    }
}

/// A three-deep 국민주택채권 book, padded to five.
///
/// The instrument's own levels are [`ktb_book`]'s; the 채권종류 levels sit at
/// visibly different prices and far larger sizes, so reading the wrong block
/// — or stepping 78 bytes instead of 156 — changes every asserted value.
pub fn nhb_book() -> Vec<SmallLotLevel> {
    let own = ktb_book();
    let class = [
        BondLevel {
            ask_price: "00010300.00",
            bid_price: "00010290.00",
            ask_qty: 9_000_000,
            bid_qty: 7_000_000,
            ask_yield: "000003.200000",
            bid_yield: "000003.210000",
        },
        BondLevel {
            ask_price: "00010310.00",
            bid_price: "00010280.00",
            ask_qty: 8_000_000,
            bid_qty: 6_000_000,
            ask_yield: "000003.190000",
            bid_yield: "000003.220000",
        },
        BondLevel {
            ask_price: "00010320.00",
            bid_price: "00010270.00",
            ask_qty: 7_000_000,
            bid_qty: 5_000_000,
            ask_yield: "000003.180000",
            bid_yield: "000003.230000",
        },
        BondLevel::EMPTY,
        BondLevel::EMPTY,
    ];
    own.into_iter().zip(class).map(|(own, class)| SmallLotLevel { own, class }).collect()
}

/// The four 총잔량 fields that close a 소액채권 book.
#[derive(Debug, Clone, Copy)]
pub struct SmallLotTotals {
    /// 채권매도/매수호가총잔량.
    pub own: (u64, u64),
    /// 채권종류매도/매수호가총잔량.
    pub class: (u64, u64),
}

impl SmallLotTotals {
    const NHB: Self = Self { own: (1_400_000, 900_000), class: (24_000_000, 18_000_000) };

    fn write(&self, buf: &mut Vec<u8>) {
        num(buf, 15, self.own.0);
        num(buf, 15, self.own.1);
        num(buf, 15, self.class.0);
        num(buf, 15, self.class.1);
    }
}

/// A `B6` 소액채권 우선호가 message (`IFMSRPD0024`).
#[derive(Debug, Clone)]
pub struct SmallLotB6 {
    /// The 41-byte header.
    pub header: BondHeader,
    /// Five levels, shallowest first.
    pub levels: Vec<SmallLotLevel>,
    /// The four closing 총잔량.
    pub totals: SmallLotTotals,
}

impl SmallLotB6 {
    /// A 국민주택채권 book.
    pub fn nhb(levels: Vec<SmallLotLevel>) -> Self {
        Self { header: BondHeader::NHB, levels, totals: SmallLotTotals::NHB }
    }

    /// Serialises to bytes, checking the total against the interface length.
    pub fn build(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.header.write(&mut buf);
        for l in &self.levels {
            l.write(&mut buf);
        }
        self.totals.write(&mut buf);
        buf.push(0xFF);
        assert_eq!(buf.len(), 882, "B6 소액채권 is IFMSRPD0024");
        buf
    }
}

/// A `G7` 소액채권 체결 + 우선호가 message (`IFMSRPD0030`).
#[derive(Debug, Clone)]
pub struct SmallLotG7 {
    /// The 41-byte header.
    pub header: BondHeader,
    /// The shared trade block.
    pub trade: BondTradeBlock,
    /// Five levels, shallowest first.
    pub levels: Vec<SmallLotLevel>,
    /// The four closing 총잔량.
    pub totals: SmallLotTotals,
}

impl SmallLotG7 {
    /// A 국민주택채권 print with the book it left.
    pub fn nhb(levels: Vec<SmallLotLevel>) -> Self {
        Self {
            header: BondHeader { trcode: "G701M", ..BondHeader::NHB },
            trade: BondTradeBlock::default(),
            levels,
            totals: SmallLotTotals::NHB,
        }
    }

    /// Serialises to bytes, checking the total against the interface length.
    pub fn build(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.header.write(&mut buf);
        self.trade.write(&mut buf);
        for l in &self.levels {
            l.write(&mut buf);
        }
        self.totals.write(&mut buf);
        buf.push(0xFF);
        assert_eq!(buf.len(), 1063, "G7 소액채권 is IFMSRPD0030");
        buf
    }
}
