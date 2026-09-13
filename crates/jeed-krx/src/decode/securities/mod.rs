//! What 주식 and ETF·ELW·ETN messages share.
//!
//! The 증권 market opens its real-time messages with the same 47-byte shape-A
//! header the derivative market uses ([`common`](super::common)), but nothing
//! below that is the same: prices are eleven bytes rather than nine, quantities
//! twelve rather than nine, and **there are no order counts**.
//!
//! ## Two book shapes, one market
//!
//! The split is by whether the product has a liquidity provider, and it is
//! clean — no product group sends both:
//!
//! | 전문 | 상품군 | 레벨 | 단수 |
//! |---|---|--:|--:|
//! | `B6` `IFMSRPD0002` 590 B | `01S` `01Q` `01X` 주식 | 46 B | 10 |
//! | `B7` `IFMSRPD0003` 830 B | `02S` ELW · `03S` ETF · `04S` ETN · `05S` 수익증권 | 70 B | 10 |
//!
//! `B603S` does not exist and neither does `B701S`. The extra 24 bytes per
//! level on `B7` are the LP quantities, which ride in each level's `ext` word
//! ([`level_ext::LP_QUANTITY`]).
//!
//! ## 체결 is one interface for both
//!
//! `A3` `IFMSRPD0004` (186 B) is sent for 주식 and ETF alike, so it is decoded
//! once here and re-exported by both markets
//! ([`stock::trade`](super::stock::trade), [`etf::trade`](super::etf::trade))
//! rather than copied into each.

pub mod trade;

use crate::decode::common::{BookAccum, BookShape, slice};
use crate::error::KrxError;
use crate::extract::KRX;
use crate::field;
use jeed_wire::{QuotePayload, WIRE_MAX_DEPTH, WireLevel, level_ext};

/// Both 증권 우선호가 interfaces carry ten levels a side.
pub const DEPTH: usize = 10;

const _: () = assert!(DEPTH <= WIRE_MAX_DEPTH);

/// 증권 가격 field width — `[부호][미사용][유효숫자 9]`.
pub const PRICE_LEN: usize = 11;

/// 증권 잔량 field width.
pub const QTY_LEN: usize = 12;

// Offsets within one level block.
const LVL_ASK_PRICE: usize = 0;
const LVL_BID_PRICE: usize = 11;
const LVL_ASK_QTY: usize = 22;
const LVL_BID_QTY: usize = 34;
const LVL_ASK_LP_QTY: usize = 46;
const LVL_BID_LP_QTY: usize = 58;

/// How one book level is spelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelShape {
    /// Bytes per level.
    pub stride: usize,

    /// Whether the level carries LP quantities behind the ordinary ones.
    pub lp: bool,
}

/// `B6` — price and quantity a side, 46 bytes.
pub const PLAIN: LevelShape = LevelShape { stride: 46, lp: false };

/// `B7` — the same plus LP quantities, 70 bytes.
pub const WITH_LP: LevelShape = LevelShape { stride: 70, lp: true };

const _: () = assert!(PLAIN.stride == 2 * PRICE_LEN + 2 * QTY_LEN);
const _: () = assert!(WITH_LP.stride == PLAIN.stride + 2 * QTY_LEN);

/// Bytes after the last level block on both interfaces.
///
/// 총잔량 ×2, 예상체결가, 예상체결수량, 중간가격, 중간가호가총잔량 ×2, `0xFF`.
pub const TAIL_LEN: usize = 83;

/// Offset of 예상체결가 relative to the end of the level blocks.
pub const TAIL_EXPECTED_PRICE: usize = 24;

const _: () = assert!(TAIL_EXPECTED_PRICE == 2 * QTY_LEN);

/// Fills [`DEPTH`] book levels starting at `first_level`.
///
/// 증권 channels carry **no order counts**, so `ORDER_COUNT_VALID` is left
/// clear and every level's count stays zero. That is a different statement from
/// "zero orders rest here", and it is why the flag is payload-level.
pub fn fill_book(
    payload: &[u8],
    first_level: usize,
    shape: LevelShape,
    out: &mut QuotePayload,
) -> Result<BookShape, KrxError> {
    let price = &KRX.securities_price;
    let mut accum = BookAccum::default();

    for level in 0..DEPTH {
        let at = first_level + level * shape.stride;

        let ask_price =
            field::price(price, slice(payload, at + LVL_ASK_PRICE, PRICE_LEN), at + LVL_ASK_PRICE)?;
        let bid_price =
            field::price(price, slice(payload, at + LVL_BID_PRICE, PRICE_LEN), at + LVL_BID_PRICE)?;
        let ask_qty = field::uint(slice(payload, at + LVL_ASK_QTY, QTY_LEN), at + LVL_ASK_QTY)?;
        let bid_qty = field::uint(slice(payload, at + LVL_BID_QTY, QTY_LEN), at + LVL_BID_QTY)?;

        let ask_qty = ask_qty.unwrap_or(0);
        let bid_qty = bid_qty.unwrap_or(0);
        accum.observe(level, ask_qty, bid_qty);

        let mut ask = WireLevel::new(ask_price.unwrap_or(0), ask_qty);
        let mut bid = WireLevel::new(bid_price.unwrap_or(0), bid_qty);

        if shape.lp {
            let ask_lp =
                field::uint(slice(payload, at + LVL_ASK_LP_QTY, QTY_LEN), at + LVL_ASK_LP_QTY)?;
            let bid_lp =
                field::uint(slice(payload, at + LVL_BID_LP_QTY, QTY_LEN), at + LVL_BID_LP_QTY)?;
            // The LP's share of the level, not a quantity on top of it — the
            // ordinary 잔량 already includes it.
            ask.ext = ask_lp.unwrap_or(0) as u32;
            bid.ext = bid_lp.unwrap_or(0) as u32;
        }

        out.set_ask(level, ask);
        out.set_bid(level, bid);
    }

    if shape.lp {
        out.with_level_ext(level_ext::LP_QUANTITY);
    }

    Ok(accum.finish())
}

/// Reads 예상체결가 out of the tail and, if an auction is actually running,
/// puts it in the payload's spare word.
///
/// Zero means no auction. The four aggregate fields around it — 총잔량 ×2,
/// 중간가격, 중간가호가총잔량 ×2 — have no wire slot; see `documents/todo.md`.
pub fn fill_expected_price(
    payload: &[u8],
    tail: usize,
    out: &mut QuotePayload,
) -> Result<(), KrxError> {
    let at = tail + TAIL_EXPECTED_PRICE;
    let expected = field::price(&KRX.securities_price, slice(payload, at, PRICE_LEN), at)?;
    if let Some(v) = expected
        && v != 0
    {
        out.with_quote_ext(jeed_wire::quote_ext::EXPECTED_PRICE, v as u64);
    }
    Ok(())
}
