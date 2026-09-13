//! `@bookTicker` — Binance USD-M top of book.
//!
//! ```json
//! {"e":"bookTicker","u":400900217,"E":1568014460893,"T":1568014460891,
//!  "s":"BNBUSDT","b":"25.35190000","B":"31.21000000",
//!  "a":"25.36520000","A":"40.66000000"}
//! ```
//!
//! Spot's [`bbo`](crate::binance::spot::bbo) plus a clock: `T` is when the
//! order book changed, `E` is when the event was pushed. `T` is the one that
//! belongs in `venue_ns`; `E` measures Binance's own publishing latency and
//! has no wire slot.

use crate::binance::seen;
use crate::book::BookShape;
use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{next_key, parse_string_bytes, parse_u64, skip_value};
use crate::mask::require;
use crate::millis_to_nanos;
use jeed_wire::{QuotePayload, UnixNano, WireKind, WireLevel, WireRecord, quote_ext};

/// Every field the record needs.
const REQUIRED: u8 = seen::BID_PRICE | seen::BID_QTY | seen::ASK_PRICE | seen::ASK_QTY;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[
    (seen::BID_PRICE, "b"),
    (seen::BID_QTY, "B"),
    (seen::ASK_PRICE, "a"),
    (seen::ASK_QTY, "A"),
];

/// Decodes a USD-M `@bookTicker` frame into `out`.
///
/// `out` is left untouched on error.
pub fn decode(
    inst: &Instrument,
    payload: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut bid = WireLevel::default();
    let mut ask = WireLevel::default();
    let mut update_id: u64 = 0;
    let mut txn_ms: Option<u64> = None;
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
            b'T' => {
                let (v, next) = parse_u64(payload, pos);
                txn_ms = Some(v);
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
    if let Some(ms) = txn_ms {
        h.set_venue_time(millis_to_nanos(ms));
    }

    *out = WireRecord::new_quote(h, quote);
    Ok(())
}
