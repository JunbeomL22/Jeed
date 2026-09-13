//! `/api/v4/spot/order_book` — a whole Gate book, from the REST body.
//!
//! ```json
//! {"id":123456,"current":1606295412123,"update":1606295412100,
//!  "asks":[["27000.50","8.760"]],"bids":[["27000.00","2.710"]]}
//! ```
//!
//! | key | |
//! |---|---|
//! | `id` | the book's update id → `quote_ext` [`SEQUENCE`] |
//! | `update` | when the book last changed (ms) → `venue_ns` |
//! | `current` | when the response was built — not carried |
//! | `asks` `bids` | `[[price, amount], …]`, best first |
//!
//! ## This is the other half of the delta channel
//!
//! `id` is in the same number space as the WS channel's `u`, which is what
//! makes a resynchronisation possible at all: the consumer takes this book,
//! then applies every buffered diff whose `u` is greater than `id`. Carrying
//! `id` is therefore not bookkeeping — it is the whole reason a snapshot is
//! worth publishing rather than just holding.
//!
//! It is also why the request has to be made with `with_id=true`. Without it
//! Gate answers the same book with no `id` at all, and this decodes it as a
//! book with no sequence: usable as a picture, useless as a starting point.
//!
//! ## Nothing here fetches
//!
//! This crate owns no socket. It decodes a body the caller already has — see
//! [`bitget::snapshot`](crate::bitget::snapshot) for why the decode belongs on
//! this side of the ring at all.
//!
//! [`SEQUENCE`]: jeed_wire::quote_ext::SEQUENCE

use crate::book::BookShape;
use crate::error::CryptoError;
use crate::gate::seen;
use crate::instrument::Instrument;
use crate::json::{next_field, parse_scalar_u64, quote_levels, skip_value};
use crate::mask::require;
use crate::time::millis_to_nanos;
use jeed_wire::{QuotePayload, UnixNano, WireKind, WireRecord, quote_ext};

/// Every field the record needs. Not `id`: a book with no sequence is still a
/// book, and an absent extension says the chain is unavailable.
const REQUIRED: u8 = seen::BIDS | seen::ASKS;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[(seen::BIDS, "bids"), (seen::ASKS, "asks")];

/// Decodes a REST order-book body into `out`.
///
/// `out` is left untouched on error.
pub fn decode(
    inst: &Instrument,
    payload: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut quote = QuotePayload::default();
    let mut id: Option<u64> = None;
    let mut update_ms: Option<u64> = None;
    let mut bid_depth = 0u8;
    let mut ask_depth = 0u8;
    let mut have = 0u8;

    let mut pos = 0;
    while pos < payload.len() {
        let (key, next) = next_field(payload, pos);
        pos = next;
        if key.is_empty() {
            break;
        }

        match key {
            b"bids" => {
                let (d, next) = quote_levels(inst, payload, pos, &mut quote.bid)?;
                bid_depth = d;
                have |= seen::BIDS;
                pos = next;
            }
            b"asks" => {
                let (d, next) = quote_levels(inst, payload, pos, &mut quote.ask)?;
                ask_depth = d;
                have |= seen::ASKS;
                pos = next;
            }
            b"id" => {
                let (v, next) = parse_scalar_u64(payload, pos);
                id = Some(v);
                pos = next;
            }
            b"update" => {
                let (v, next) = parse_scalar_u64(payload, pos);
                update_ms = Some(v);
                pos = next;
            }
            _ => pos = skip_value(payload, pos),
        }
    }
    require(have, REQUIRED, KEYS)?;

    if let Some(id) = id {
        quote.with_quote_ext(quote_ext::SEQUENCE, id);
    }

    let shape = BookShape::new(bid_depth, ask_depth);
    let mut h = inst.header(WireKind::Quote, recv_ns);
    h.set_depth(shape.depth).set_flags(shape.flags);
    if let Some(ms) = update_ms {
        h.set_venue_time(millis_to_nanos(ms));
    }

    *out = WireRecord::new_quote(h, quote);
    Ok(())
}
