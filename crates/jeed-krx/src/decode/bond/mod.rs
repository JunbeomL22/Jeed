//! 일반채권·국고채권 — `01B` 장내일반채권, `01K` 장내국채.
//!
//! ```text
//! B6  IFMSRPD0023  462 B  우선호가
//! A3  IFMSRPD0027  223 B  체결
//! G7  IFMSRPD0029  643 B  체결 + 우선호가
//! ```
//!
//! `G7` is `A3`'s body followed by `B6`'s book, so the trade block
//! ([`fill_trade`]) and the level block ([`fill_book`]) are each written once
//! and used from both.
//!
//! ## Three things are not like the other markets
//!
//! **The header is 41 bytes, not 47.** 채권 carries no 정보분배종목인덱스, so
//! the clock field sits directly behind the 종목코드 — shape B in
//! [`common`](super::common). Reading it with the shape-A reader shifts every
//! field after the ISIN by six.
//!
//! **Every level carries a yield.** Price and yield are two views of the same
//! quote and the exchange sends both; the yield rides in each level's `ext`
//! word ([`level_ext::BOND_YIELD`]).
//!
//! **Quantities are 천원, not units.** That is a unit, not a scale — the wire's
//! `qty_scale` stays `S0` and the number means what the exchange says it means.
//! A consumer comparing 채권 size against 주식 size without knowing that is
//! comparing face value to share count.
//!
//! 소액채권 (`IFMSRPD0024`/`0030`) and REPO (`0025`/`0031`) are different, much
//! larger interfaces and are out of scope; see `documents/todo.md`.

pub mod quote;
pub mod trade;
pub mod trade_quote;

use crate::decode::common::{BookAccum, BookShape, Header, slice};
use crate::error::KrxError;
use crate::extract::KRX;
use crate::field;
use crate::trcode::TrCode;
use jeed_convert::ParseErr;
use jeed_wire::{
    BookYield, ISIN_LEN, QuotePayload, TradePayload, WIRE_MAX_DEPTH, WireLevel, level_ext,
    trade_kind,
};

/// Shape B — bytes before the body on 채권 real-time messages.
pub const HEADER_LEN: usize = 41;

// Shape B field offsets (documents/krx/layouts.md).
const OFF_SEQUENCE: usize = 5;
const OFF_BOARD: usize = 13;
const OFF_SESSION: usize = 15;
/// 종목코드 offset — shape B. Same place as shape A even though the header is
/// six bytes shorter; the 정보분배종목인덱스 that 채권 lacks sits *after* it.
pub const OFF_ISIN: usize = 17;
const OFF_TIME: usize = 29;

/// Both 채권 우선호가 forms carry five levels a side.
pub const DEPTH: usize = 5;

const _: () = assert!(DEPTH <= WIRE_MAX_DEPTH);

/// 채권 가격 field width — same `[부호][미사용][유효숫자 9]` shape 증권 uses.
pub const PRICE_LEN: usize = 11;

/// 채권 잔량 field width. The unit is 천원.
pub const QTY_LEN: usize = 15;

/// 채권 수익률 field width — `[부호][정수 5][.][소수 6]`.
pub const YIELD_LEN: usize = 13;

/// Bytes per book level.
pub const LEVEL_LEN: usize = 78;

const _: () = assert!(LEVEL_LEN == 2 * PRICE_LEN + 2 * QTY_LEN + 2 * YIELD_LEN);

// Offsets within one level block.
const LVL_ASK_PRICE: usize = 0;
const LVL_BID_PRICE: usize = 11;
const LVL_ASK_QTY: usize = 22;
const LVL_BID_QTY: usize = 37;
const LVL_ASK_YIELD: usize = 52;
const LVL_BID_YIELD: usize = 65;

/// Bytes after the last level block — 채권매도/매수호가총잔량 and `0xFF`.
pub const BOOK_TAIL_LEN: usize = 31;

const _: () = assert!(BOOK_TAIL_LEN == 2 * QTY_LEN + 1);

/// Reads a shape-B header (채권). Assumes the frame check has already run.
pub fn header(payload: &[u8]) -> Result<Header, KrxError> {
    if payload.len() < HEADER_LEN {
        return Err(KrxError::TooShort { need: HEADER_LEN, got: payload.len() });
    }

    Ok(Header {
        trcode: TrCode::from_message(payload)?,
        sequence: field::uint(slice(payload, OFF_SEQUENCE, 8), OFF_SEQUENCE)?,
        board: [payload[OFF_BOARD], payload[OFF_BOARD + 1]],
        session: [payload[OFF_SESSION], payload[OFF_SESSION + 1]],
        isin: field::isin(slice(payload, OFF_ISIN, ISIN_LEN))?,
        // 채권 does not carry one. Absent, not zero — the wire never sees it
        // either way, but "the field was not there" and "the field said zero"
        // are not the same claim.
        index: None,
        time_of_day_ns: field::time_of_day_ns(slice(payload, OFF_TIME, 12), OFF_TIME)?,
    })
}

/// Narrows a 수익률 to the wire's `BookYield`.
///
/// Six decimal places, so `3.125%` is `3_125_000`. The field has room for five
/// integer digits, which would not fit — no bond trades at 99999%, but a
/// corrupt message could say so, and a silent wrap would put a plausible yield
/// on the wire.
#[inline]
fn narrow_yield(value: i64, at: usize) -> Result<BookYield, KrxError> {
    BookYield::try_from(value).map_err(|_| KrxError::Field { at, err: ParseErr::Overflow })
}

/// Fills [`DEPTH`] book levels starting at `first_level`.
pub fn fill_book(
    payload: &[u8],
    first_level: usize,
    out: &mut QuotePayload,
) -> Result<BookShape, KrxError> {
    let price = &KRX.bond_price;
    let yield_reader = &KRX.bond_yield;
    let mut accum = BookAccum::default();

    for level in 0..DEPTH {
        let at = first_level + level * LEVEL_LEN;

        let ask_price =
            field::price(price, slice(payload, at + LVL_ASK_PRICE, PRICE_LEN), at + LVL_ASK_PRICE)?;
        let bid_price =
            field::price(price, slice(payload, at + LVL_BID_PRICE, PRICE_LEN), at + LVL_BID_PRICE)?;
        let ask_qty = field::uint(slice(payload, at + LVL_ASK_QTY, QTY_LEN), at + LVL_ASK_QTY)?;
        let bid_qty = field::uint(slice(payload, at + LVL_BID_QTY, QTY_LEN), at + LVL_BID_QTY)?;
        let ask_yield = field::price(
            yield_reader,
            slice(payload, at + LVL_ASK_YIELD, YIELD_LEN),
            at + LVL_ASK_YIELD,
        )?;
        let bid_yield = field::price(
            yield_reader,
            slice(payload, at + LVL_BID_YIELD, YIELD_LEN),
            at + LVL_BID_YIELD,
        )?;

        let ask_qty = ask_qty.unwrap_or(0);
        let bid_qty = bid_qty.unwrap_or(0);
        accum.observe(level, ask_qty, bid_qty);

        let mut ask = WireLevel::new(ask_price.unwrap_or(0), ask_qty);
        let mut bid = WireLevel::new(bid_price.unwrap_or(0), bid_qty);
        ask.ext = narrow_yield(ask_yield.unwrap_or(0), at + LVL_ASK_YIELD)? as u32;
        bid.ext = narrow_yield(bid_yield.unwrap_or(0), at + LVL_BID_YIELD)? as u32;

        out.set_ask(level, ask);
        out.set_bid(level, bid);
    }

    out.with_level_ext(level_ext::BOND_YIELD);
    Ok(accum.finish())
}

// Trade block offsets, from documents/krx/layouts.md. `A3` (IFMSRPD0027) and
// `G7` (IFMSRPD0029) spell it identically.
const OFF_PRICE: usize = 41;
const OFF_QTY: usize = 52;
const OFF_TRADE_YIELD: usize = 92;
const OFF_CUMULATIVE_QTY: usize = 177;

/// End of the 채권 trade block — where `G7`'s book starts and where `A3`'s
/// 결제일자 ends.
pub const TRADE_BLOCK_END: usize = 222;

/// Reads the 채권 trade block at `[41:222]`.
///
/// There is **no aggressor field**: 채권 체결 does not say which side took, so
/// `trade_kind` is [`trade_kind::NONE`] — "this channel has no such field" —
/// rather than `UNKNOWN`, which would mean the field was there and said
/// nothing.
pub fn fill_trade(payload: &[u8]) -> Result<TradePayload, KrxError> {
    let price = field::price(&KRX.bond_price, slice(payload, OFF_PRICE, PRICE_LEN), OFF_PRICE)?;
    let qty = field::uint(slice(payload, OFF_QTY, 10), OFF_QTY)?;
    let cumulative =
        field::uint(slice(payload, OFF_CUMULATIVE_QTY, QTY_LEN), OFF_CUMULATIVE_QTY)?;
    let trade_yield = field::price(
        &KRX.bond_yield,
        slice(payload, OFF_TRADE_YIELD, YIELD_LEN),
        OFF_TRADE_YIELD,
    )?;

    let mut trade = TradePayload::new(price.unwrap_or(0), qty.unwrap_or(0));
    if let Some(c) = cumulative {
        trade.with_cumulative_qty(c);
    }
    if let Some(y) = trade_yield {
        trade.with_yield(narrow_yield(y, OFF_TRADE_YIELD)?);
    }
    trade.with_kind(trade_kind::NONE);

    Ok(trade)
}

/// `true` for the two 채권 product groups this module decodes.
///
/// 소액채권 `01M` and REPO `01R` are deliberately absent: they use different,
/// much larger interfaces.
#[inline]
pub const fn is_bond_group(trcode: TrCode) -> bool {
    matches!(trcode.product_group(), [b'0', b'1', b'B'] | [b'0', b'1', b'K'])
}
