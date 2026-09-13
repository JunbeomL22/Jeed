//! The parts every KRX real-time message shares.
//!
//! ## There is not one header, there are three
//!
//! The standard draws each interface as its own table, so the shared opening is
//! easy to miss — and easy to over-generalise. Measured against
//! `documents/krx/layouts.md`:
//!
//! | 모양 | 길이 | 쓰는 곳 | 다른 점 |
//! |---|--:|---|---|
//! | A | 47 B | 파생·증권 `B6`/`B7`/`A3`/`G7` | 기준형 |
//! | B | 41 B | 채권 `B6`/`A3`/`G7` | 정보분배종목인덱스 없음 |
//! | C | 33 B | 파생 `V1`/`Q2` | 세션ID 없음, 인덱스는 종목코드 뒤 |
//!
//! Shape A is the one three markets share, so it lives here as [`header()`].
//! The other two are built by the modules that own them — the bond decoders and
//! [`derivative::limit_header`](crate::decode::derivative::limit_header) — out
//! of the same [`Header`](struct@Header) struct, so everything downstream sees
//! one type.

use crate::error::KrxError;
use crate::trcode::TrCode;
use crate::field::{ISIN_LEN, Isin};
use jeed_wire::{RecordHeader, Scale, UnixNano};

/// Shape A — bytes before the body on 파생·증권 real-time messages.
pub const HEADER_LEN: usize = 47;

// Shape A field offsets (documents/krx/layouts.md).
const OFF_SEQUENCE: usize = 5;
const OFF_BOARD: usize = 13;
const OFF_SESSION: usize = 15;
/// 종목코드 offset — shape A. Public because the receive loop reads the ISIN
/// off a datagram it has not decoded yet, to drop unwanted instruments before
/// claiming a ring slot ([`dispatch::isin_offset`](crate::decode::dispatch::isin_offset)).
pub const OFF_ISIN: usize = 17;
const OFF_INDEX: usize = 29;
const OFF_TIME: usize = 35;

/// The fields that open a KRX real-time message, whichever shape it is.
///
/// Fields a given shape does not carry are filled with their "absent" value
/// rather than left to the caller to remember: 채권 has no 정보분배종목인덱스,
/// `V1`/`Q2` have no 세션ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// The message's own trcode.
    pub trcode: TrCode,

    /// 정보분배일련번호. **`None` when the field is blank**, which whole
    /// channels do — `B606F` sent nothing but blanks until 2026-04-01, and
    /// `V103F` still does. Blank means "not measurable", not "no gap", and it
    /// is per (instrument × board) so it is not a channel liveness signal
    /// either (`CLAUDE.md`).
    pub sequence: Option<u64>,

    /// 보드ID.
    pub board: [u8; 2],

    /// 세션ID, or two spaces on a shape that has none.
    pub session: [u8; 2],

    /// 종목코드.
    pub isin: Isin,

    /// 정보분배종목인덱스 — a per-day, per-market index, absent on 채권. Not
    /// carried on the wire: it is not stable across days, so the consumer
    /// resolves identity from `(venue, isin)` instead.
    pub index: Option<u64>,

    /// 매매처리시각 as nanoseconds since KST midnight, dateless.
    pub time_of_day_ns: Option<u64>,
}

/// Reads a shape-A header (파생·증권). Assumes the frame check has already run.
pub fn header(payload: &[u8]) -> Result<Header, KrxError> {
    if payload.len() < HEADER_LEN {
        return Err(KrxError::TooShort { need: HEADER_LEN, got: payload.len() });
    }

    let trcode = TrCode::from_message(payload)?;
    let sequence = crate::field::uint(slice(payload, OFF_SEQUENCE, 8), OFF_SEQUENCE)?;
    let isin = crate::field::isin(slice(payload, OFF_ISIN, ISIN_LEN))?;
    let index = crate::field::uint(slice(payload, OFF_INDEX, 6), OFF_INDEX)?;
    let time_of_day_ns =
        crate::field::time_of_day_ns(slice(payload, OFF_TIME, 12), OFF_TIME)?;

    Ok(Header {
        trcode,
        sequence,
        board: [payload[OFF_BOARD], payload[OFF_BOARD + 1]],
        session: [payload[OFF_SESSION], payload[OFF_SESSION + 1]],
        isin,
        index,
        time_of_day_ns,
    })
}

/// `payload[at..at + len]`, in a `const fn`.
#[inline]
pub(crate) const fn slice(payload: &[u8], at: usize, len: usize) -> &[u8] {
    let (_, rest) = payload.split_at(at);
    let (field, _) = rest.split_at(len);
    field
}

/// Fills the wire header of a record built from `msg`.
///
/// `venue_ns` is only marked valid when the message actually spelled a time:
/// a blank 매매처리시각 is "not measurable", and the consumer must not age a
/// book against a zero.
#[inline]
pub fn fill_record_header(
    h: &mut RecordHeader,
    msg: &Header,
    recv_ns: UnixNano,
    price_scale: Scale,
) {
    h.symbol = crate::field::wire_symbol(&msg.isin);
    h.recv_ns = recv_ns;
    if let Some(tod) = msg.time_of_day_ns {
        h.set_venue_time(crate::clock::absolute_ns(tod, recv_ns));
    }
    h.set_scales(price_scale, Scale::S0);
}

/// What filling a book told us about it.
///
/// Not the price scale: that comes from the reader the decoder chose for the
/// instrument ([`crate::extract`]), so it is known before the first byte is
/// read and there is nothing to discover by walking the levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BookShape {
    /// Levels actually carrying a resting order, per the deeper side.
    pub depth: u8,

    /// [`header_flags::BID_EMPTY`](jeed_wire::header_flags::BID_EMPTY) /
    /// [`header_flags::ASK_EMPTY`](jeed_wire::header_flags::ASK_EMPTY) as they
    /// apply.
    pub flags: u8,
}

/// Accumulates the facts a book fill discovers, so every market's fill loop
/// reports depth and emptiness the same way.
///
/// Depth is **counted, not assumed**: levels past the end of the real book
/// arrive zero-filled rather than blank, so a five-deep channel carrying two
/// resting levels reports two.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct BookAccum {
    ask_depth: usize,
    bid_depth: usize,
}

impl BookAccum {
    /// Records one level's quantities.
    #[inline]
    pub(crate) fn observe(&mut self, level: usize, ask_qty: u64, bid_qty: u64) {
        if ask_qty > 0 {
            self.ask_depth = level + 1;
        }
        if bid_qty > 0 {
            self.bid_depth = level + 1;
        }
    }

    /// Turns the accumulated facts into a [`BookShape`].
    #[inline]
    pub(crate) fn finish(self) -> BookShape {
        let mut flags = 0u8;
        if self.ask_depth == 0 {
            flags |= jeed_wire::header_flags::ASK_EMPTY;
        }
        if self.bid_depth == 0 {
            flags |= jeed_wire::header_flags::BID_EMPTY;
        }
        BookShape { depth: self.ask_depth.max(self.bid_depth) as u8, flags }
    }
}
