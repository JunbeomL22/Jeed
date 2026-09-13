//! `@bookTicker` — Binance spot top of book.
//!
//! ```json
//! {"u":400900217,"s":"BNBUSDT","b":"25.35190000","B":"31.21000000",
//!  "a":"25.36520000","A":"40.66000000"}
//! ```
//!
//! | key | |
//! |---|---|
//! | `u` | order book update id → `quote_ext` as [`quote_ext::SEQUENCE`] |
//! | `s` | symbol — checked against the decoder's instrument, then dropped |
//! | `b` `B` | best bid price / quantity |
//! | `a` `A` | best ask price / quantity |
//!
//! **Spot carries no venue timestamp.** USD-M's `@bookTicker` has `T`; this
//! one has nothing, so [`header_flags::VENUE_TIME_VALID`] stays clear and the
//! consumer ages the quote against `recv_ns` alone. Leaving `venue_ns` zero
//! and unflagged is the point: a zero that claimed to be a time would make
//! every quote look decades stale.
//!
//! [`quote_ext::SEQUENCE`]: jeed_wire::quote_ext::SEQUENCE
//! [`header_flags::VENUE_TIME_VALID`]: jeed_wire::header_flags::VENUE_TIME_VALID

use crate::binance::seen;
use crate::book::BookShape;
use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{next_key, parse_string_bytes, parse_u64, skip_value};
use crate::mask::require;
use jeed_wire::{
    QuotePayload, UnixNano, WireKind, WireLevel, WireRecord, quote_ext,
};

/// Every field the record needs.
const REQUIRED: u8 = seen::BID_PRICE | seen::BID_QTY | seen::ASK_PRICE | seen::ASK_QTY;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[
    (seen::BID_PRICE, "b"),
    (seen::BID_QTY, "B"),
    (seen::ASK_PRICE, "a"),
    (seen::ASK_QTY, "A"),
];

/// Decodes a `@bookTicker` frame into `out`.
///
/// `out` is left untouched on error: the record is assembled on the stack and
/// installed only once every field has parsed, so a failure cannot leave a
/// half-filled record for the ring to publish (`CLAUDE.md`).
pub fn decode(
    inst: &Instrument,
    payload: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut bid = WireLevel::default();
    let mut ask = WireLevel::default();
    let mut update_id: u64 = 0;
    let mut have = 0u8;

    let mut pos = 0;
    while pos < payload.len() {
        let (key, next) = next_key(payload, pos);
        pos = next;
        if key == 0 {
            break;
        }

        match key {
            b'u' => {
                let (v, next) = parse_u64(payload, pos);
                update_id = v;
                pos = next;
            }
            b's' => {
                let (s, next) = parse_string_bytes(payload, pos);
                inst.check_symbol(s)?;
                pos = next;
            }
            b'b' => {
                let (v, next) = parse_string_bytes(payload, pos);
                bid.price = inst.price(v, "b")?;
                have |= seen::BID_PRICE;
                pos = next;
            }
            b'B' => {
                let (v, next) = parse_string_bytes(payload, pos);
                bid.qty = inst.qty(v, "B")?;
                have |= seen::BID_QTY;
                pos = next;
            }
            b'a' => {
                let (v, next) = parse_string_bytes(payload, pos);
                ask.price = inst.price(v, "a")?;
                have |= seen::ASK_PRICE;
                pos = next;
            }
            b'A' => {
                let (v, next) = parse_string_bytes(payload, pos);
                ask.qty = inst.qty(v, "A")?;
                have |= seen::ASK_QTY;
                pos = next;
            }
            _ => pos = skip_value(payload, pos),
        }
    }
    require(have, REQUIRED, KEYS)?;

    let mut quote = QuotePayload::default();
    quote.set_bid(0, bid).set_ask(0, ask);
    quote.with_quote_ext(quote_ext::SEQUENCE, update_id);

    let shape = BookShape::top_of_book(bid.qty, ask.qty);
    let mut h = inst.header(WireKind::Quote, recv_ns);
    h.set_depth(shape.depth).set_flags(shape.flags);

    *out = WireRecord::new_quote(h, quote);
    Ok(())
}
