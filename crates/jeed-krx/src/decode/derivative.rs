//! Derivative real-time messages (`DRV`, product groups ending in `F`).
//!
//! Every one of them opens with the same 47-byte header and, where it carries a
//! book, the same 46-byte level block repeated `depth` times. That regularity
//! is not an accident of the spec — it is why one decoder covers both the
//! five-deep and ten-deep variants of a message rather than two near-copies
//! drifting apart:
//!
//! | | 5-deep | 10-deep |
//! |---|---|---|
//! | `B6` 우선호가 | `IFMSRPD0034` 324 B | `IFMSRPD0035` 554 B |
//! | `G7` 체결+우선호가 | `IFMSRPD0037` 431 B | `IFMSRPD0038` 661 B |
//!
//! Offsets below come from `documents/krx/layouts.md`, which is generated from
//! the spec. They are not counted by hand — the spec's own offset column is an
//! *end* offset, and reading it as a start shifts every field by one while
//! still producing plausible numbers.

pub mod quote;
pub mod trade_quote;

use crate::error::KrxError;
use crate::field::{self, Decimal};
use crate::trcode::TrCode;
use jeed_wire::{
    ISIN_LEN, Isin, QuotePayload, Scale, UnixNano, WIRE_MAX_DEPTH, WireLevel, header_flags,
};

/// Bytes before the first book level (and before the trade block on `G7`).
pub const HEADER_LEN: usize = 47;

/// Bytes per book level: ask price, bid price, ask qty, bid qty, ask count,
/// bid count.
pub const LEVEL_LEN: usize = 46;

// Header field offsets (documents/krx/layouts.md).
const OFF_SEQUENCE: usize = 5;
const OFF_BOARD: usize = 13;
const OFF_SESSION: usize = 15;
const OFF_ISIN: usize = 17;
const OFF_INDEX: usize = 29;
const OFF_TIME: usize = 35;

// Offsets within one level block.
const LVL_ASK_PRICE: usize = 0;
const LVL_BID_PRICE: usize = 9;
const LVL_ASK_QTY: usize = 18;
const LVL_BID_QTY: usize = 27;
const LVL_ASK_COUNT: usize = 36;
const LVL_BID_COUNT: usize = 41;

/// The eight fields that open every derivative real-time message.
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

    /// 세션ID.
    pub session: [u8; 2],

    /// 종목코드.
    pub isin: Isin,

    /// 정보분배종목인덱스 — a per-day, per-market index. Not carried on the
    /// wire: it is not stable across days, so the consumer resolves identity
    /// from `(venue, isin)` instead.
    pub index: Option<u64>,

    /// 매매처리시각 as nanoseconds since KST midnight, dateless.
    pub time_of_day_ns: Option<u64>,
}

/// Reads the common header. Assumes the frame check has already run.
pub const fn header(payload: &[u8]) -> Result<Header, KrxError> {
    if payload.len() < HEADER_LEN {
        return Err(KrxError::TooShort { need: HEADER_LEN, got: payload.len() });
    }

    let trcode = match TrCode::from_message(payload) {
        Ok(c) => c,
        Err(e) => return Err(e),
    };
    let sequence = match field::uint(slice(payload, OFF_SEQUENCE, 8), OFF_SEQUENCE) {
        Ok(v) => v,
        Err(e) => return Err(e),
    };
    let isin = match field::isin(slice(payload, OFF_ISIN, ISIN_LEN)) {
        Ok(v) => v,
        Err(e) => return Err(e),
    };
    let index = match field::uint(slice(payload, OFF_INDEX, 6), OFF_INDEX) {
        Ok(v) => v,
        Err(e) => return Err(e),
    };
    let time_of_day_ns = match field::time_of_day_ns(slice(payload, OFF_TIME, 12), OFF_TIME) {
        Ok(v) => v,
        Err(e) => return Err(e),
    };

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

/// What filling a book told us about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookShape {
    /// Price scale read off the message (the decimal point's position varies by
    /// instrument, not by message — see [`Decimal`]).
    pub price_scale: Scale,

    /// Levels actually carrying a resting order, per the deeper side.
    pub depth: u8,

    /// [`header_flags::BID_EMPTY`] / [`header_flags::ASK_EMPTY`] as they apply.
    pub flags: u8,
}

/// Fills `depth` book levels starting at `first_level`, and reports what the
/// book turned out to be.
///
/// Levels past the end of the real book arrive zero-filled, so the depth the
/// wire reports is counted rather than assumed: a five-deep channel with two
/// resting levels says two.
pub fn fill_book(
    payload: &[u8],
    first_level: usize,
    depth: usize,
    out: &mut QuotePayload,
) -> Result<BookShape, KrxError> {
    debug_assert!(depth <= WIRE_MAX_DEPTH);

    let mut decimals: Option<u8> = None;
    let mut ask_depth = 0usize;
    let mut bid_depth = 0usize;

    for level in 0..depth {
        let at = first_level + level * LEVEL_LEN;

        let ask_price = field::decimal(slice(payload, at + LVL_ASK_PRICE, 9), at + LVL_ASK_PRICE)?;
        let bid_price = field::decimal(slice(payload, at + LVL_BID_PRICE, 9), at + LVL_BID_PRICE)?;
        let ask_qty = field::uint(slice(payload, at + LVL_ASK_QTY, 9), at + LVL_ASK_QTY)?;
        let bid_qty = field::uint(slice(payload, at + LVL_BID_QTY, 9), at + LVL_BID_QTY)?;
        let ask_count = field::uint(slice(payload, at + LVL_ASK_COUNT, 5), at + LVL_ASK_COUNT)?;
        let bid_count = field::uint(slice(payload, at + LVL_BID_COUNT, 5), at + LVL_BID_COUNT)?;

        // The first price that is spelled at all fixes the scale for the whole
        // message: every price in one message is one instrument's.
        if decimals.is_none() {
            decimals = ask_price.or(bid_price).map(|d: Decimal| d.decimals);
        }

        let ask_qty = ask_qty.unwrap_or(0);
        let bid_qty = bid_qty.unwrap_or(0);
        if ask_qty > 0 {
            ask_depth = level + 1;
        }
        if bid_qty > 0 {
            bid_depth = level + 1;
        }

        out.set_ask(
            level,
            WireLevel::with_count(
                ask_price.map_or(0, |d| d.value),
                ask_qty,
                ask_count.unwrap_or(0) as u32,
            ),
        );
        out.set_bid(
            level,
            WireLevel::with_count(
                bid_price.map_or(0, |d| d.value),
                bid_qty,
                bid_count.unwrap_or(0) as u32,
            ),
        );
    }

    // Derivative channels always carry order counts; equity ones do not, which
    // is why this is a payload-level fact and not a per-level one.
    out.with_order_counts();

    let mut flags = 0u8;
    if ask_depth == 0 {
        flags |= header_flags::ASK_EMPTY;
    }
    if bid_depth == 0 {
        flags |= header_flags::BID_EMPTY;
    }

    let decimals = decimals.unwrap_or(0);
    let price_scale = Scale::from_decimals(decimals as usize)
        .ok_or(KrxError::Overflow { at: first_level })?;

    Ok(BookShape {
        price_scale,
        depth: ask_depth.max(bid_depth) as u8,
        flags,
    })
}

/// Fills the header of a record that is about to carry a derivative message.
///
/// `venue_ns` is only marked valid when the message actually spelled a time:
/// a blank 매매처리시각 is "not measurable", and the consumer must not age a
/// book against a zero.
#[inline]
pub fn fill_record_header(
    h: &mut jeed_wire::RecordHeader,
    msg: &Header,
    recv_ns: UnixNano,
    price_scale: Scale,
) {
    h.isin = msg.isin;
    h.recv_ns = recv_ns;
    if let Some(tod) = msg.time_of_day_ns {
        h.set_venue_time(crate::clock::absolute_ns(tod, recv_ns));
    }
    h.set_scales(price_scale, Scale::S0);
}
