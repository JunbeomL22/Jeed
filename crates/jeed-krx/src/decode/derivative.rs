//! Derivative real-time messages (`DRV`, product groups ending in `F`).
//!
//! Every one of them opens with the 47-byte shape-A header
//! ([`common`](super::common)) and, where it carries a book, the same 46-byte
//! level block repeated `depth` times. That regularity is not an accident of
//! the spec — it is why one decoder covers both the five-deep and ten-deep
//! variants of a message rather than two near-copies drifting apart:
//!
//! | | 5-deep | 10-deep |
//! |---|---|---|
//! | `B6` 우선호가 | `IFMSRPD0034` 324 B | `IFMSRPD0035` 554 B |
//! | `G7` 체결+우선호가 | `IFMSRPD0037` 431 B | `IFMSRPD0038` 661 B |
//!
//! `A3` 체결 (`IFMSRPD0036`, 173 B) is the same trade block as `G7` with the
//! book cut off, which is why [`fill_trade`] is shared rather than copied.
//!
//! The two price-limit messages (`V1`, `Q2`) use a different, shorter header —
//! [`limit_header`].
//!
//! Offsets below come from `documents/krx/layouts.md`, which is generated from
//! the spec. They are not counted by hand — the spec's own offset column is an
//! *end* offset, and reading it as a start shifts every field by one while
//! still producing plausible numbers.

pub mod dynamic_limit;
pub mod price_limit;
pub mod quote;
pub mod trade;
pub mod trade_quote;

use crate::decode::common::{BookAccum, BookShape, Header, slice};
use crate::error::KrxError;
use crate::field::{self, Decimal};
use crate::trcode::TrCode;
use jeed_wire::{ISIN_LEN, QuotePayload, Scale, TradePayload, WIRE_MAX_DEPTH, WireLevel, trade_kind};

pub use crate::decode::common::{HEADER_LEN, fill_record_header, header};

/// Bytes per book level: ask price, bid price, ask qty, bid qty, ask count,
/// bid count.
pub const LEVEL_LEN: usize = 46;

// Offsets within one level block.
const LVL_ASK_PRICE: usize = 0;
const LVL_BID_PRICE: usize = 9;
const LVL_ASK_QTY: usize = 18;
const LVL_BID_QTY: usize = 27;
const LVL_ASK_COUNT: usize = 36;
const LVL_BID_COUNT: usize = 41;

/// Bytes before the body on `V1` / `Q2` — shape C. Shorter than shape A
/// because these two carry no 세션ID, and the 정보분배종목인덱스 sits directly
/// behind the 종목코드 instead of in front of the timestamp.
pub const LIMIT_HEADER_LEN: usize = 33;

// Shape C offsets.
const LIM_SEQUENCE: usize = 5;
const LIM_BOARD: usize = 13;
const LIM_ISIN: usize = 15;
const LIM_INDEX: usize = 27;

/// Book depth of a derivative product group.
///
/// Only the single-stock **option** families are ten deep:
///
/// | 상품군 | | 단수 |
/// |---|---|---|
/// | `05F` | 주식옵션 | 10 |
/// | `18F` | 개별주식 위클리옵션 | 10 |
/// | 그 외 | | 5 |
///
/// `04F` (주식선물) is deliberately **not** in that list. The book is ten deep
/// at the exchange but the feed truncates it to five, and nothing downstream
/// uses more (`CLAUDE.md`). The generated table's `[derivative_depth]` section
/// is the same statement, taken from the channel standard.
#[inline]
pub const fn depth_for_product_group(trcode: TrCode) -> usize {
    match trcode.product_group() {
        [b'0', b'5', b'F'] | [b'1', b'8', b'F'] => 10,
        _ => 5,
    }
}

/// Reads a shape-C header (`V1` / `Q2`).
///
/// `time_at` / `time_len` locate the message's own clock field, which the two
/// do not agree on: `V1` spells 가격확대시각 in nine bytes (milliseconds), `Q2`
/// spells 매매처리시각 in twelve (microseconds).
pub const fn limit_header(
    payload: &[u8],
    time_at: usize,
    time_len: usize,
) -> Result<Header, KrxError> {
    if payload.len() < time_at + time_len {
        return Err(KrxError::TooShort { need: time_at + time_len, got: payload.len() });
    }

    let trcode = match TrCode::from_message(payload) {
        Ok(c) => c,
        Err(e) => return Err(e),
    };
    let sequence = match field::uint(slice(payload, LIM_SEQUENCE, 8), LIM_SEQUENCE) {
        Ok(v) => v,
        Err(e) => return Err(e),
    };
    let isin = match field::isin(slice(payload, LIM_ISIN, ISIN_LEN)) {
        Ok(v) => v,
        Err(e) => return Err(e),
    };
    let index = match field::uint(slice(payload, LIM_INDEX, 6), LIM_INDEX) {
        Ok(v) => v,
        Err(e) => return Err(e),
    };
    let time_of_day_ns = match field::time_of_day_ns(slice(payload, time_at, time_len), time_at) {
        Ok(v) => v,
        Err(e) => return Err(e),
    };

    Ok(Header {
        trcode,
        sequence,
        board: [payload[LIM_BOARD], payload[LIM_BOARD + 1]],
        session: [b' ', b' '],
        isin,
        index,
        time_of_day_ns,
    })
}

/// Fills `depth` book levels starting at `first_level`, and reports what the
/// book turned out to be.
pub fn fill_book(
    payload: &[u8],
    first_level: usize,
    depth: usize,
    out: &mut QuotePayload,
) -> Result<BookShape, KrxError> {
    debug_assert!(depth <= WIRE_MAX_DEPTH);

    let mut accum = BookAccum::default();

    for level in 0..depth {
        let at = first_level + level * LEVEL_LEN;

        let ask_price = field::decimal(slice(payload, at + LVL_ASK_PRICE, 9), at + LVL_ASK_PRICE)?;
        let bid_price = field::decimal(slice(payload, at + LVL_BID_PRICE, 9), at + LVL_BID_PRICE)?;
        let ask_qty = field::uint(slice(payload, at + LVL_ASK_QTY, 9), at + LVL_ASK_QTY)?;
        let bid_qty = field::uint(slice(payload, at + LVL_BID_QTY, 9), at + LVL_BID_QTY)?;
        let ask_count = field::uint(slice(payload, at + LVL_ASK_COUNT, 5), at + LVL_ASK_COUNT)?;
        let bid_count = field::uint(slice(payload, at + LVL_BID_COUNT, 5), at + LVL_BID_COUNT)?;

        let ask_qty = ask_qty.unwrap_or(0);
        let bid_qty = bid_qty.unwrap_or(0);
        accum.observe(level, ask_price.or(bid_price).map(|d: Decimal| d.decimals), ask_qty, bid_qty);

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

    accum.finish(first_level)
}

// Offsets of the trade block, from documents/krx/layouts.md. `A3`
// (IFMSRPD0036) and `G7` (IFMSRPD0037/0038) spell it identically — `A3` is
// `G7` with the book removed — so these are stated once.
const OFF_PRICE: usize = 47;
const OFF_QTY: usize = 56;
const OFF_CUMULATIVE_QTY: usize = 119;
const OFF_AGGRESSOR: usize = 153;
const OFF_DYN_UPPER: usize = 154;
const OFF_DYN_LOWER: usize = 163;

/// End of the derivative trade block — where `G7`'s book starts and where
/// `A3`'s end keyword sits.
pub const TRADE_BLOCK_END: usize = 172;

/// Reads the derivative trade block at `[47:172]`.
///
/// Shared by `A3` and `G7` because the two spell it byte for byte alike: the
/// print, the running totals, the aggressor code and the dynamic band.
pub fn fill_trade(payload: &[u8]) -> Result<TradePayload, KrxError> {
    let price = field::decimal(slice(payload, OFF_PRICE, 9), OFF_PRICE)?;
    let qty = field::uint(slice(payload, OFF_QTY, 9), OFF_QTY)?;
    let cumulative = field::uint(slice(payload, OFF_CUMULATIVE_QTY, 12), OFF_CUMULATIVE_QTY)?;
    let dyn_upper = field::decimal(slice(payload, OFF_DYN_UPPER, 9), OFF_DYN_UPPER)?;
    let dyn_lower = field::decimal(slice(payload, OFF_DYN_LOWER, 9), OFF_DYN_LOWER)?;

    let mut trade = TradePayload::new(price.map_or(0, |d| d.value), qty.unwrap_or(0));

    if let Some(c) = cumulative {
        trade.with_cumulative_qty(c);
    }

    // 매도매수구분코드: space 단일가체결 / '0' 해당없음 / '1' 매도 / '2' 매수.
    // The first two are both "there was no aggressor" — an auction cross has
    // none by definition — so they collapse to UNKNOWN. That is distinct from
    // NONE, which means the channel has no such field at all.
    trade.with_kind(match payload[OFF_AGGRESSOR] {
        b'1' => trade_kind::SELL,
        b'2' => trade_kind::BUY,
        _ => trade_kind::UNKNOWN,
    });

    // KRX cannot say "not applicable": instruments outside the dynamic limit
    // regime (far-month futures, spreads) carry `000000.00` rather than blanks.
    // A real band always has both sides above zero, so that is the test — and
    // the answer rides in a flag so the consumer never has to make it again.
    if let (Some(u), Some(l)) = (dyn_upper, dyn_lower)
        && u.value > 0
        && l.value > 0
    {
        trade.with_dyn_limits(u.value, l.value);
    }

    Ok(trade)
}

/// Price scale of a trade block read on its own, where there is no book to
/// read it from.
///
/// The scale is a property of the instrument, not the message, so any price
/// field spells it; 체결가격 is the one always present.
#[inline]
pub fn trade_price_scale(payload: &[u8]) -> Result<Scale, KrxError> {
    let price = field::decimal(slice(payload, OFF_PRICE, 9), OFF_PRICE)?;
    let decimals = price.map_or(0, |d| d.decimals);
    Scale::from_decimals(decimals as usize).ok_or(KrxError::Overflow { at: OFF_PRICE })
}
