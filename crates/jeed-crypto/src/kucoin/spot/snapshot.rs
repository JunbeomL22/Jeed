//! `/api/v3/market/orderbook/level2` — a whole KuCoin spot book, from the
//! REST body.
//!
//! ```json
//! {"code":"200000","data":{"time":1602997267139,"sequence":"1602997267139",
//!                          "bids":[["3535.5","0.03"]],
//!                          "asks":[["3536.1","0.1"]]}}
//! ```
//!
//! | key | |
//! |---|---|
//! | `sequence` | the book's sequence → `quote_ext` [`SEQUENCE`] |
//! | `time` | when the book was taken, in **milliseconds** → `venue_ns` |
//! | `bids` `asks` | `[[price, size], …]`, best first |
//! | `code` | envelope — not read, see [`bitget::snapshot`] |
//!
//! ## This is the other half of the delta channel
//!
//! `sequence` is in the same number space as the WS channel's
//! `sequenceStart`/`sequenceEnd`, so the consumer takes this book and then
//! applies every buffered diff whose `sequenceEnd` is past it. Carrying it is
//! what makes the snapshot a starting point rather than a picture.
//!
//! ## Milliseconds here, nanoseconds on the trade channel
//!
//! The same word — `time` — means different units on the two KuCoin spot
//! endpoints. This one is milliseconds and is multiplied;
//! [`trade`](crate::kucoin::spot::trade)'s is nanoseconds and is not. They are
//! six orders of magnitude apart, so the mistake would be visible rather than
//! subtle — but only if someone looked.
//!
//! [`SEQUENCE`]: jeed_wire::quote_ext::SEQUENCE
//! [`bitget::snapshot`]: crate::bitget::snapshot

use crate::book::BookShape;
use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{next_field, object_at, parse_scalar_u64, quote_levels, skip_value};
use crate::kucoin::seen;
use crate::mask::require;
use crate::time::millis_to_nanos;
use jeed_wire::{QuotePayload, UnixNano, WireKind, WireRecord, quote_ext};

/// Every field the record needs.
const REQUIRED: u8 = seen::BIDS | seen::ASKS;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[(seen::BIDS, "bids"), (seen::ASKS, "asks")];

/// Decodes a REST level2 body into `out`.
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
    let mut sequence: Option<u64> = None;
    let mut time_ms: Option<u64> = None;
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
            b"sequence" => {
                let (v, next) = parse_scalar_u64(data, pos);
                sequence = Some(v);
                pos = next;
            }
            b"time" => {
                let (v, next) = parse_scalar_u64(data, pos);
                time_ms = Some(v);
                pos = next;
            }
            _ => pos = skip_value(data, pos),
        }
    }
    require(have, REQUIRED, KEYS)?;

    if let Some(sequence) = sequence {
        quote.with_quote_ext(quote_ext::SEQUENCE, sequence);
    }

    let shape = BookShape::new(bid_depth, ask_depth);
    let mut h = inst.header(WireKind::Quote, recv_ns);
    h.set_depth(shape.depth).set_flags(shape.flags);
    if let Some(ms) = time_ms {
        h.set_venue_time(millis_to_nanos(ms));
    }

    *out = WireRecord::new_quote(h, quote);
    Ok(())
}
