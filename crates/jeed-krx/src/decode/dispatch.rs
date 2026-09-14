//! Choosing a decoder from the five trcode bytes.
//!
//! ```text
//!   B 6 0 4 F
//!   └─┬─┘ └┬─┘
//!     │     └── 정보구분+시장구분 → 시장, 그리고 파생이면 호가 단수
//!     └──────── 데이터구분        → 무슨 전문인지
//! ```
//!
//! **Two levels, data class first.** The first two bytes say what kind of
//! message it is and the last three say whose. Neither half is enough alone:
//! `B6` spans eight interfaces from 324 to 1387 bytes, and `04F` appears under
//! `A3`, `B6`, `G7`, `B2` and more. Branching on the data class first is what
//! keeps the second level small — one `match` per kind over a handful of
//! product groups, instead of one flat table of every code KRX defines.
//!
//! ## An unknown trcode is not an error to hide
//!
//! [`decode`] returns [`KrxError::UnknownTrCode`] rather than silently doing
//! nothing, because the two have different causes: a code this build does not
//! decode is a configuration question (why is that channel joined?), while a
//! code that does not exist is a corrupt datagram. The receive loop counts them
//! separately from decode failures.

use crate::decode::{bond, common, derivative, etf, schedule, securities, stock};
use crate::error::KrxError;
use crate::trcode::TrCode;
use jeed_wire::{UnixNano, WireRecord};

/// Decodes one datagram into `out`, picking the decoder from its trcode.
///
/// `out` is left untouched on any error, including an unknown trcode.
///
/// The trcode is read from the payload rather than taken as an argument: the
/// caller has already read it to decide whether the channel is wanted, and a
/// decoder re-reading it is not duplicated work but the decoder refusing to
/// apply one interface's layout to another's bytes.
pub fn decode(payload: &[u8], recv_ns: UnixNano, out: &mut WireRecord) -> Result<(), KrxError> {
    let trcode = TrCode::from_message(payload)?;

    match trcode.data_class() {
        [b'B', b'6'] => quote(trcode, payload, recv_ns, out),
        [b'B', b'7'] if etf::quote::handles(trcode) => {
            etf::quote::DECODER.decode(payload, recv_ns, out)
        }
        [b'A', b'3'] => trade(trcode, payload, recv_ns, out),
        [b'G', b'7'] => trade_quote(trcode, payload, recv_ns, out),
        [b'V', b'1'] if derivative::price_limit::handles(trcode) => {
            derivative::price_limit::DECODER.decode(payload, recv_ns, out)
        }
        [b'Q', b'2'] if derivative::dynamic_limit::handles(trcode) => {
            derivative::dynamic_limit::DECODER.decode(payload, recv_ns, out)
        }
        [b'M', b'4'] => schedule::DECODER.decode(payload, recv_ns, out),
        _ => Err(KrxError::UnknownTrCode { code: trcode }),
    }
}

/// `B6` — 우선호가. Five markets share the data class and not one byte of
/// layout: 324/554 B 파생, 590 B 주식, 462 B 채권, 882 B 소액채권.
fn quote(
    trcode: TrCode,
    payload: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), KrxError> {
    if trcode.is_derivative() {
        // The depth is the product group's, and 주식선물 is the trap: a
        // ten-deep product the feed truncates to five (`CLAUDE.md`).
        return match derivative::depth_for_product_group(trcode) {
            10 => derivative::quote::TEN_DEEP.decode(payload, recv_ns, out),
            _ => derivative::quote::FIVE_DEEP.decode(payload, recv_ns, out),
        };
    }
    if stock::quote::handles(trcode) {
        return stock::quote::DECODER.decode(payload, recv_ns, out);
    }
    if bond::quote::handles(trcode) {
        return bond::quote::DECODER.decode(payload, recv_ns, out);
    }
    if bond::small_lot::quote::handles(trcode) {
        return bond::small_lot::quote::DECODER.decode(payload, recv_ns, out);
    }
    Err(KrxError::UnknownTrCode { code: trcode })
}

/// `A3` — 체결. One interface per market, none of them the same length.
fn trade(
    trcode: TrCode,
    payload: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), KrxError> {
    if derivative::trade::handles(trcode) {
        return derivative::trade::DECODER.decode(payload, recv_ns, out);
    }
    // 주식 and the LP products are one interface, so this is the one place the
    // two market modules collapse back together.
    if securities::trade::handles(trcode) {
        return securities::trade::DECODER.decode(payload, recv_ns, out);
    }
    if bond::trade::handles(trcode) {
        return bond::trade::DECODER.decode(payload, recv_ns, out);
    }
    Err(KrxError::UnknownTrCode { code: trcode })
}

/// `G7` — 체결 + 우선호가. 증권 does not send one; its 체결 carries a
/// price-only top of book instead, which is not a book.
fn trade_quote(
    trcode: TrCode,
    payload: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), KrxError> {
    if trcode.is_derivative() {
        return match derivative::depth_for_product_group(trcode) {
            10 => derivative::trade_quote::TEN_DEEP.decode(payload, recv_ns, out),
            _ => derivative::trade_quote::FIVE_DEEP.decode(payload, recv_ns, out),
        };
    }
    if bond::trade_quote::handles(trcode) {
        return bond::trade_quote::DECODER.decode(payload, recv_ns, out);
    }
    if bond::small_lot::trade_quote::handles(trcode) {
        return bond::small_lot::trade_quote::DECODER.decode(payload, recv_ns, out);
    }
    Err(KrxError::UnknownTrCode { code: trcode })
}

/// The message length `decode` will require for this trcode, if it decodes it.
///
/// The receive loop uses this to size a datagram check before it commits a ring
/// slot, so a message of the wrong length costs nothing but a comparison.
pub const fn message_len(trcode: TrCode) -> Option<usize> {
    let deep = derivative::depth_for_product_group(trcode) == 10;
    Some(match trcode.data_class() {
        [b'B', b'6'] if trcode.is_derivative() => {
            if deep {
                derivative::quote::TEN_DEEP.message_len()
            } else {
                derivative::quote::FIVE_DEEP.message_len()
            }
        }
        [b'G', b'7'] if trcode.is_derivative() => {
            if deep {
                derivative::trade_quote::TEN_DEEP.message_len()
            } else {
                derivative::trade_quote::FIVE_DEEP.message_len()
            }
        }
        [b'A', b'3'] if derivative::trade::handles(trcode) => derivative::trade::MESSAGE_LEN,
        [b'V', b'1'] if derivative::price_limit::handles(trcode) => {
            derivative::price_limit::MESSAGE_LEN
        }
        [b'Q', b'2'] if derivative::dynamic_limit::handles(trcode) => {
            derivative::dynamic_limit::MESSAGE_LEN
        }
        [b'B', b'6'] if stock::quote::handles(trcode) => stock::quote::MESSAGE_LEN,
        [b'B', b'7'] if etf::quote::handles(trcode) => etf::quote::MESSAGE_LEN,
        [b'A', b'3'] if securities::trade::handles(trcode) => securities::trade::MESSAGE_LEN,
        [b'B', b'6'] if bond::quote::handles(trcode) => bond::quote::MESSAGE_LEN,
        [b'A', b'3'] if bond::trade::handles(trcode) => bond::trade::MESSAGE_LEN,
        [b'G', b'7'] if bond::trade_quote::handles(trcode) => bond::trade_quote::MESSAGE_LEN,
        [b'B', b'6'] if bond::small_lot::quote::handles(trcode) => {
            bond::small_lot::quote::MESSAGE_LEN
        }
        [b'G', b'7'] if bond::small_lot::trade_quote::handles(trcode) => {
            bond::small_lot::trade_quote::MESSAGE_LEN
        }
        [b'M', b'4'] => schedule::MESSAGE_LEN,
        _ => return None,
    })
}

/// Where the 종목코드 sits in a datagram of this trcode, if it has one.
///
/// The receive loop uses it to apply the ISIN allow-set **before** it claims a
/// ring slot: an options product-group port carries every strike, and decoding
/// a strike nobody trades only to throw it away is the one filter cost that is
/// avoidable.
///
/// `None` means *the allow-set does not apply*, not *no instrument*. `M4`
/// carries a 종목코드 but its subject is the board — a market-wide halt names
/// no instrument, and filtering the schedule channel by instrument would drop
/// exactly the message that matters most.
pub const fn isin_offset(trcode: TrCode) -> Option<usize> {
    match trcode.data_class() {
        // Shapes A and B put it in the same place; the assertion below is the
        // reason this can be one arm.
        [b'B', b'6'] | [b'B', b'7'] | [b'A', b'3'] | [b'G', b'7'] => Some(common::OFF_ISIN),
        [b'V', b'1'] | [b'Q', b'2'] => Some(derivative::LIM_ISIN),
        _ => None,
    }
}

// 채권 drops 정보분배종목인덱스 from the header, but from *behind* the
// 종목코드 — so the offset survives and `isin_offset` needs no market branch.
// If a standard revision ever moves one of them, this fails the build rather
// than filtering on six bytes of 종목코드 and six of something else.
const _: () = assert!(bond::OFF_ISIN == common::OFF_ISIN);

/// `true` if this build decodes the trcode.
///
/// Startup uses it to warn about a configured channel nothing will read, which
/// is a warning rather than a refusal: the distribution standard lags the live
/// feed, so a legitimate new code can be absent from the table
/// (`documents/todo.md`).
#[inline]
pub const fn handles(trcode: TrCode) -> bool {
    message_len(trcode).is_some()
}
