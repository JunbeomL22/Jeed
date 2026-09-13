//! Message builders for the decoder tests.
//!
//! Field widths here are the spec's, so a builder that produces a message of
//! the wrong length is itself the bug report: every builder asserts its own
//! total against the interface length before returning.

#![allow(dead_code)]

/// Appends a fixed-width field, asserting the caller spelled it at full width.
///
/// KRX does not pad — a nine-byte price field holds nine bytes — so a test that
/// writes `"937.00"` where `"000937.00"` belongs should fail loudly rather than
/// shift every field after it.
pub fn put(buf: &mut Vec<u8>, width: usize, text: &str) {
    assert_eq!(text.len(), width, "field {text:?} is not {width} bytes");
    buf.extend_from_slice(text.as_bytes());
}

/// Appends a zero-padded unsigned field.
pub fn num(buf: &mut Vec<u8>, width: usize, value: u64) {
    let s = format!("{value:0width$}");
    assert_eq!(s.len(), width, "{value} does not fit in {width} bytes");
    buf.extend_from_slice(s.as_bytes());
}

/// Appends `width` spaces — a blank field, which is not the same as a zero one.
pub fn blank(buf: &mut Vec<u8>, width: usize) {
    buf.extend(std::iter::repeat_n(b' ', width));
}

/// One book level as the wire spells it.
#[derive(Debug, Clone, Copy)]
pub struct Level {
    /// 매도 price, exactly nine bytes (`"000937.05"`).
    pub ask_price: &'static str,
    /// 매수 price, exactly nine bytes.
    pub bid_price: &'static str,
    /// 매도 잔량.
    pub ask_qty: u64,
    /// 매수 잔량.
    pub bid_qty: u64,
    /// 매도 주문건수.
    pub ask_count: u64,
    /// 매수 주문건수.
    pub bid_count: u64,
}

impl Level {
    /// An empty level: KRX sends zeros, not blanks, past the end of the book.
    pub const EMPTY: Self = Self {
        ask_price: "000000.00",
        bid_price: "000000.00",
        ask_qty: 0,
        bid_qty: 0,
        ask_count: 0,
        bid_count: 0,
    };

    /// A level with prices, quantities and one order a side.
    pub const fn new(ask_price: &'static str, bid_price: &'static str, qty: u64) -> Self {
        Self { ask_price, bid_price, ask_qty: qty, bid_qty: qty, ask_count: 1, bid_count: 1 }
    }

    fn write(&self, buf: &mut Vec<u8>) {
        put(buf, 9, self.ask_price);
        put(buf, 9, self.bid_price);
        num(buf, 9, self.ask_qty);
        num(buf, 9, self.bid_qty);
        num(buf, 5, self.ask_count);
        num(buf, 5, self.bid_count);
    }
}

/// The 47-byte header every derivative real-time message opens with.
#[derive(Debug, Clone)]
pub struct Header {
    /// 데이터구분값 + 정보구분값, five bytes.
    pub trcode: &'static str,
    /// 정보분배일련번호. `None` writes eight spaces, which whole channels do.
    pub sequence: Option<u64>,
    /// 보드ID.
    pub board: &'static str,
    /// 세션ID.
    pub session: &'static str,
    /// 종목코드, twelve bytes.
    pub isin: &'static str,
    /// 정보분배종목인덱스.
    pub index: u64,
    /// 매매처리시각 `HHMMSSuuuuuu`.
    pub time: &'static str,
}

impl Header {
    /// A KOSPI200 futures header at 09:01:00.123456.
    pub const KOSPI200_FUTURES: Self = Self {
        trcode: "B601F",
        sequence: Some(1),
        board: "G1",
        session: "40",
        isin: "KR4101V90009",
        index: 1,
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
        num(buf, 6, self.index);
        put(buf, 12, self.time);
        assert_eq!(buf.len(), 47, "derivative header is 47 bytes");
    }
}

/// A `B6` 파생 우선호가 message (`IFMSRPD0034` / `IFMSRPD0035`).
#[derive(Debug, Clone)]
pub struct B6 {
    /// The common header.
    pub header: Header,
    /// Levels, shallowest first. Its length is the interface's depth.
    pub levels: Vec<Level>,
    /// 매도호가총잔량 / 매수호가총잔량.
    pub totals: (u64, u64),
    /// 매도호가유효건수 / 매수호가유효건수.
    pub counts: (u64, u64),
    /// 예상체결가, nine bytes.
    pub expected_price: &'static str,
    /// 예상체결수량.
    pub expected_qty: u64,
}

impl B6 {
    /// A five-deep KOSPI200 futures book.
    pub fn kospi200(levels: Vec<Level>) -> Self {
        Self {
            header: Header::KOSPI200_FUTURES,
            levels,
            totals: (0, 0),
            counts: (0, 0),
            expected_price: "000000.00",
            expected_qty: 0,
        }
    }

    /// Serialises to bytes, checking the total against the interface length.
    pub fn build(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.header.write(&mut buf);
        for l in &self.levels {
            l.write(&mut buf);
        }
        num(&mut buf, 9, self.totals.0);
        num(&mut buf, 9, self.totals.1);
        num(&mut buf, 5, self.counts.0);
        num(&mut buf, 5, self.counts.1);
        put(&mut buf, 9, self.expected_price);
        num(&mut buf, 9, self.expected_qty);
        buf.push(0xFF);

        let expected = if self.levels.len() == 5 { 324 } else { 554 };
        assert_eq!(buf.len(), expected, "B6 with {} levels", self.levels.len());
        buf
    }
}

/// A `G7` 파생 체결 + 우선호가 message (`IFMSRPD0037` / `IFMSRPD0038`).
#[derive(Debug, Clone)]
pub struct G7 {
    /// The common header.
    pub header: Header,
    /// 체결가격, nine bytes.
    pub price: &'static str,
    /// 거래량.
    pub qty: u64,
    /// 근월물체결가격 / 원월물체결가격, nine bytes each.
    pub spread_legs: (&'static str, &'static str),
    /// 시가 / 고가 / 저가 / 직전가격, nine bytes each.
    pub session_prices: (&'static str, &'static str, &'static str, &'static str),
    /// 누적거래량.
    pub cumulative_qty: u64,
    /// 누적거래대금, twenty-two bytes.
    pub cumulative_value: &'static str,
    /// 최종매도매수구분코드: `b' '` 단일가체결 / `b'0'` 해당없음 / `b'1'` 매도 /
    /// `b'2'` 매수.
    pub aggressor: u8,
    /// 동적상한가 / 동적하한가, nine bytes each. `"000000.00"` is how KRX
    /// spells "the regime does not cover this instrument".
    pub dyn_limits: (&'static str, &'static str),
    /// Levels, shallowest first.
    pub levels: Vec<Level>,
    /// 매도호가총잔량 / 매수호가총잔량.
    pub totals: (u64, u64),
    /// 매도호가유효건수 / 매수호가유효건수.
    pub counts: (u64, u64),
}

impl G7 {
    /// A five-deep KOSPI200 futures print at 937.00 with a ±1% dynamic band.
    pub fn kospi200(levels: Vec<Level>) -> Self {
        Self {
            header: Header { trcode: "G701F", ..Header::KOSPI200_FUTURES },
            price: "000937.00",
            qty: 3,
            spread_legs: ("000000.00", "000000.00"),
            session_prices: ("000935.10", "000938.40", "000933.75", "000936.95"),
            cumulative_qty: 166_478,
            cumulative_value: "000000000155908543.000",
            aggressor: b'2',
            dyn_limits: ("000946.35", "000927.65"),
            levels,
            totals: (0, 0),
            counts: (0, 0),
        }
    }

    /// Serialises to bytes, checking the total against the interface length.
    pub fn build(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.header.write(&mut buf);
        put(&mut buf, 9, self.price);
        num(&mut buf, 9, self.qty);
        put(&mut buf, 9, self.spread_legs.0);
        put(&mut buf, 9, self.spread_legs.1);
        put(&mut buf, 9, self.session_prices.0);
        put(&mut buf, 9, self.session_prices.1);
        put(&mut buf, 9, self.session_prices.2);
        put(&mut buf, 9, self.session_prices.3);
        num(&mut buf, 12, self.cumulative_qty);
        put(&mut buf, 22, self.cumulative_value);
        buf.push(self.aggressor);
        put(&mut buf, 9, self.dyn_limits.0);
        put(&mut buf, 9, self.dyn_limits.1);
        assert_eq!(buf.len(), 172, "G7 trade block ends at 172");

        for l in &self.levels {
            l.write(&mut buf);
        }
        num(&mut buf, 9, self.totals.0);
        num(&mut buf, 9, self.totals.1);
        num(&mut buf, 5, self.counts.0);
        num(&mut buf, 5, self.counts.1);
        buf.push(0xFF);

        let expected = if self.levels.len() == 5 { 431 } else { 661 };
        assert_eq!(buf.len(), expected, "G7 with {} levels", self.levels.len());
        buf
    }
}

/// Five levels around 937.00, tightest first.
pub fn kospi200_book() -> Vec<Level> {
    vec![
        Level { ask_price: "000937.05", bid_price: "000936.95", ask_qty: 10, bid_qty: 8, ask_count: 3, bid_count: 2 },
        Level { ask_price: "000937.10", bid_price: "000936.90", ask_qty: 25, bid_qty: 31, ask_count: 7, bid_count: 9 },
        Level { ask_price: "000937.15", bid_price: "000936.85", ask_qty: 40, bid_qty: 44, ask_count: 11, bid_count: 12 },
        Level { ask_price: "000937.20", bid_price: "000936.80", ask_qty: 55, bid_qty: 60, ask_count: 15, bid_count: 16 },
        Level { ask_price: "000937.25", bid_price: "000936.75", ask_qty: 70, bid_qty: 77, ask_count: 19, bid_count: 20 },
    ]
}

/// 09:01:00.123456 KST on 2026-07-31, as Unix nanoseconds.
pub const RECV_NS: u64 = 1_785_456_060_200_000_000;

/// What 매매처리시각 `090100123456` assembles to for that reception time.
pub const VENUE_NS: u64 = 1_785_456_060_123_456_000;
