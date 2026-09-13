//! The field readers KRX messages are parsed with, built once at startup.
//!
//! Every price on a KRX feed is fixed width with the decimal point — when there
//! is one — always in the same byte. That is what lets [`jeed_convert`] parse it
//! without looking at it: the point's index is configuration, the parse is a
//! handful of bitwise operations, and the scale never travels alongside the
//! value.
//!
//! ## `integer_size` counts the sign byte
//!
//! This is the trap in [`jeed_convert::Config`], and it is silent in the worst way.
//! With `is_signed = true` the sign byte is added to `total_size` but **not** to the
//! window `clip` reads, so a nine-byte field configured as `(signed, 5 int,
//! 2 frac)` is read as its first **eight** bytes:
//!
//! ```text
//!   000937.05        the field
//!   00093 7.0 5      (signed, 5, 2)  → reads [0..8], expects '.' at index 5
//!   000937.05        (signed, 6, 2)  → reads [0..9], expects '.' at index 6  ✓
//! ```
//!
//! So the counts below are **sign + integer digits**, and `total_size` comes
//! out one larger than the field. Nothing here uses `total_size`; widths come
//! from `documents/krx/layouts.md`, which is generated.
//!
//! ## Choosing the wrong shape cannot pass quietly
//!
//! A derivative price is nine bytes in three shapes and the product group does
//! not settle which ([`derivative_price()`]). Picking wrong is therefore a real
//! risk — but not a silent one: each shape expects a `'.'` at a different index
//! and a digit everywhere else, so applying the wrong one lands a `'.'` where a
//! digit belongs (or the reverse) and the parse fails. A mis-selected shape
//! loses the message; it never quietly multiplies a price by a hundred.

use crate::trcode::TrCode;
use jeed_convert::{Config, Extractor};
use jeed_wire::Isin;
use std::sync::LazyLock;

/// A signed fixed-width reader. `lead` is the **sign byte plus the integer
/// digits**; `frac` is the digits after the point, or zero for no point.
fn signed(lead: usize, frac: usize) -> Extractor {
    let mut config =
        Config::builder().with_is_signed(true).with_integer_size(lead).with_fraction_size(frac);
    config.build().expect("field shape is a constant");
    Extractor::from(config.clone())
}

/// Every price and yield shape KRX sends on the channels Jeed decodes.
#[derive(Debug)]
pub struct KrxFields {
    /// 파생 `[부호][정수 5][.][소수 2]` — 지수·국채·금리·통화 파생. 9 B.
    pub derivative_rate: Extractor,

    /// 파생 `[부호][정수 4][.][소수 3]` — 3개월무위험지표금리선물. 9 B.
    pub derivative_risk_free: Extractor,

    /// 파생 `[부호][정수 8]` — 주식선물·주식옵션·상품파생. 9 B.
    pub derivative_plain: Extractor,

    /// 증권 `[부호][미사용][유효숫자 9]` — 11 B. The unused byte is a `'0'`,
    /// so it reads as a leading zero rather than needing to be skipped.
    pub securities_price: Extractor,

    /// 채권 가격 — 11 B, the same shape 증권 uses.
    ///
    /// A separate field rather than an alias so that a call site says which
    /// market's field it is reading. If the two shapes ever diverge — and the
    /// standard revises these tables — only one of them moves.
    pub bond_price: Extractor,

    /// 채권 수익률 `[부호][정수 5][.][소수 6]` — 13 B.
    pub bond_yield: Extractor,
}

/// The readers, built on first use.
pub static KRX: LazyLock<KrxFields> = LazyLock::new(|| KrxFields {
    derivative_rate: signed(6, 2),
    derivative_risk_free: signed(5, 3),
    derivative_plain: signed(9, 0),
    securities_price: signed(11, 0),
    bond_price: signed(11, 0),
    bond_yield: signed(6, 6),
});

/// ISIN prefixes of 3개월무위험지표금리선물 (KOFR).
///
/// These are the reason the price shape cannot be a product-group table: they
/// share `06F` with 국채·금리·통화 파생, which spell prices `[5].[2]`, while
/// these spell them `[4].[3]`.
const RISK_FREE_PREFIXES: [&[u8; 6]; 2] = [b"KR4169", b"KR4A69"];

/// Product groups whose prices carry no decimal point at all —
/// 주식선물·주식옵션·개별주식 위클리옵션 and 상품파생.
const PLAIN_PRICE_GROUPS: [[u8; 3]; 5] = [*b"04F", *b"05F", *b"07F", *b"10F", *b"18F"];

/// The price reader for one derivative message.
///
/// Two lookups, in this order, because the second only applies within `06F`:
///
/// | | 조건 | 형식 |
/// |---|---|---|
/// | 1 | 상품군이 `04F`·`05F`·`07F`·`10F`·`18F` | `[부호][8]` |
/// | 2 | 종목코드가 `KR4169`/`KR4A69` | `[부호][4].[3]` |
/// | 3 | 그 외 | `[부호][5].[2]` |
#[inline]
pub fn derivative_price(trcode: TrCode, isin: &Isin) -> &'static Extractor {
    let group = trcode.product_group();
    let mut i = 0;
    while i < PLAIN_PRICE_GROUPS.len() {
        if PLAIN_PRICE_GROUPS[i][0] == group[0]
            && PLAIN_PRICE_GROUPS[i][1] == group[1]
            && PLAIN_PRICE_GROUPS[i][2] == group[2]
        {
            return &KRX.derivative_plain;
        }
        i += 1;
    }
    if RISK_FREE_PREFIXES.iter().any(|p| isin.starts_with(*p)) {
        return &KRX.derivative_risk_free;
    }
    &KRX.derivative_rate
}
