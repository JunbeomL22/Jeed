//! `/api/v2/spot/market/orderbook` · `/api/v2/mix/market/merge-depth` — a
//! whole Bitget book, from the REST body.
//!
//! ```json
//! {"code":"00000","msg":"success","requestTime":1706000000000,
//!  "data":{"asks":[["74501.6","1.2468"],["74502.0","0.5"]],
//!          "bids":[["74500.0","2.3"],["74499.5","1.0"]],
//!          "ts":"1706000000000"}}
//! ```
//!
//! | key | |
//! |---|---|
//! | `data` | the book — an **object** here, where the WS `data` is an array |
//! | `asks` `bids` | `[[price, size], …]`, best first |
//! | `ts` | when the book was taken (ms, quoted) → `venue_ns` |
//! | `code` `msg` `requestTime` | envelope — see below |
//!
//! ## Nothing here fetches
//!
//! This crate owns no socket and makes no request; it decodes a body the
//! caller already has. The body is here at all because the alternative is
//! worse: a consumer that had to parse Bitget's REST JSON to seed a book
//! would need Bitget knowledge, and keeping venue knowledge on this side of
//! the ring is the whole point of the split (`documents/feed_handler.md` §6).
//!
//! ## `code` is not checked
//!
//! A failed request answers `{"code":"40034","msg":"…"}` with no `data`, and
//! that decodes to [`CryptoError::Missing`] — the same outcome, reached by
//! reading what the message *has* rather than what it says about itself. A
//! `code` check would additionally have to be kept in step with Bitget's list
//! of success codes, which is a second thing to be wrong about.
//!
//! ## No sequence, so none is invented
//!
//! The spot and mix depth endpoints return no sequence number, so
//! `quote_ext` stays [`NONE`](jeed_wire::quote_ext::NONE). The WS `seq` and
//! this book are not in the same number space anyway, so a consumer cannot
//! chain the two — which is exactly the fact an absent extension states
//! (`CLAUDE.md`).
//!
//! ## The symbol is not in the body
//!
//! It was in the query string. There is nothing to check `s` against here, so
//! unlike every WS decoder in this crate this one takes the caller's word —
//! which is the same word it used to build the request.

use crate::bitget::seen;
use crate::book::BookShape;
use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{next_field, object_at, parse_scalar_u64, quote_levels, skip_value};
use crate::mask::require;
use crate::time::millis_to_nanos;
use jeed_wire::{QuotePayload, UnixNano, WireKind, WireRecord};

/// Every field the record needs.
const REQUIRED: u8 = seen::BIDS | seen::ASKS;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[(seen::BIDS, "bids"), (seen::ASKS, "asks")];

/// Decodes a REST depth body into `out`.
///
/// `out` is left untouched on error.
pub fn decode(
    inst: &Instrument,
    payload: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut data: Option<&[u8]> = None;

    let mut pos = 0;
    while pos < payload.len() {
        let (key, next) = next_field(payload, pos);
        pos = next;
        if key.is_empty() {
            break;
        }
        if key == b"data"
            && let Some((inner, past)) = object_at(payload, pos)
        {
            data = Some(inner);
            pos = past;
            continue;
        }
        pos = skip_value(payload, pos);
    }

    let Some(data) = data else {
        return Err(CryptoError::Missing { key: "data" });
    };
    book(inst, data, recv_ns, out)
}

/// Fills a [`WireKind::Quote`] from the `data` object.
fn book(
    inst: &Instrument,
    data: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut quote = QuotePayload::default();
    let mut ts_ms: Option<u64> = None;
    let mut bid_depth = 0u8;
    let mut ask_depth = 0u8;
    let mut have = 0u8;

    let mut pos = 0;
    while pos < data.len() {
        let (key, next) = next_field(data, pos);
        pos = next;
        if key.is_empty() {
            break;
        }

        match key {
            b"bids" => {
                let (d, next) = quote_levels(inst, data, pos, &mut quote.bid)?;
                bid_depth = d;
                have |= seen::BIDS;
                pos = next;
            }
            b"asks" => {
                let (d, next) = quote_levels(inst, data, pos, &mut quote.ask)?;
                ask_depth = d;
                have |= seen::ASKS;
                pos = next;
            }
            b"ts" => {
                let (v, next) = parse_scalar_u64(data, pos);
                ts_ms = Some(v);
                pos = next;
            }
            _ => pos = skip_value(data, pos),
        }
    }
    require(have, REQUIRED, KEYS)?;

    let shape = BookShape::new(bid_depth, ask_depth);
    let mut h = inst.header(WireKind::Quote, recv_ns);
    h.set_depth(shape.depth).set_flags(shape.flags);
    if let Some(ms) = ts_ms {
        h.set_venue_time(millis_to_nanos(ms));
    }

    *out = WireRecord::new_quote(h, quote);
    Ok(())
}
