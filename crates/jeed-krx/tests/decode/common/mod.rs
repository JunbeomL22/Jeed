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

/// 09:01:00.123456 KST on 2026-07-31, as Unix nanoseconds.
pub const RECV_NS: u64 = 1_785_456_060_200_000_000;

/// What 매매처리시각 `090100123456` assembles to for that reception time.
pub const VENUE_NS: u64 = 1_785_456_060_123_456_000;

/// The 47-byte shape-A header — 파생과 증권이 같이 쓴다.
///
/// 채권은 다르다: 정보분배종목인덱스가 없어 41바이트다
/// ([`BondHeader`]).
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

// ===========================================================================
// Per-market builders
// ===========================================================================

mod bond;
mod derivative;
mod schedule;
mod securities;

pub use bond::*;
pub use derivative::*;
pub use schedule::*;
pub use securities::*;
