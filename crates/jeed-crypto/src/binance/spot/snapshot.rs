//! `@depth<N>` and `/api/v3/depth` — a Binance spot book, whole.
//!
//! ```json
//! {"lastUpdateId":160,"bids":[["0.0024","10"]],"asks":[["0.0026","100"]]}
//! ```
//!
//! | key | |
//! |---|---|
//! | `lastUpdateId` | the book is current as of this update id → `quote_ext` as [`quote_ext::SEQUENCE`] |
//! | `bids` `asks` | `[["price","qty"], …]`, best first |
//!
//! ## One decoder for the stream and the REST endpoint
//!
//! Spot spells both the same, so `@depth10@100ms` frames and `/api/v3/depth`
//! responses come through here unchanged. The only difference is depth:
//! the stream sends 5, 10 or 20 and REST sends up to 5,000. Everything past
//! [`WIRE_MAX_DEPTH`] is **skipped without being parsed** — running 4,990
//! discarded levels through the decimal reader would be the most expensive
//! thing this crate does.
//!
//! ## `lastUpdateId` is what makes a snapshot usable for recovery
//!
//! It is the same counter the `@depth` diff stream chains on, so a consumer
//! that has lost the chain can take this record, then apply every buffered
//! delta whose `final_update_id` is greater. Carrying it is the whole reason
//! [`quote_ext::SEQUENCE`] exists.
//!
//! [`quote_ext::SEQUENCE`]: jeed_wire::quote_ext::SEQUENCE
//! [`WIRE_MAX_DEPTH`]: jeed_wire::WIRE_MAX_DEPTH

use crate::binance::seen;
use crate::book::BookShape;
use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{next_key, parse_u64, quote_levels, skip_value};
use crate::mask::require;
use jeed_wire::{QuotePayload, UnixNano, WireKind, WireRecord, quote_ext};

/// Every field the record needs.
const REQUIRED: u8 = seen::BIDS | seen::ASKS | seen::SEQUENCE;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] =
    &[(seen::BIDS, "bids"), (seen::ASKS, "asks"), (seen::SEQUENCE, "lastUpdateId")];

/// Decodes a spot depth snapshot into `out`.
///
/// `out` is left untouched on error.
pub fn decode(
    inst: &Instrument,
    payload: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut quote = QuotePayload::default();
    let mut last_update_id: u64 = 0;
    let mut bid_depth = 0u8;
    let mut ask_depth = 0u8;
    let mut have = 0u8;

    let mut pos = 0;
    while pos < payload.len() {
        let (key, next) = next_key(payload, pos);
        pos = next;
        if key == 0 {
            break;
        }

        match key {
            b'l' => {
                let (v, next) = parse_u64(payload, pos);
                last_update_id = v;
                have |= seen::SEQUENCE;
                pos = next;
            }
            b'b' => {
                let (d, next) = quote_levels(inst, payload, pos, &mut quote.bid)?;
                bid_depth = d;
                have |= seen::BIDS;
                pos = next;
            }
            b'a' => {
                let (d, next) = quote_levels(inst, payload, pos, &mut quote.ask)?;
                ask_depth = d;
                have |= seen::ASKS;
                pos = next;
            }
            _ => pos = skip_value(payload, pos),
        }
    }
    require(have, REQUIRED, KEYS)?;

    quote.with_quote_ext(quote_ext::SEQUENCE, last_update_id);

    let shape = BookShape::new(bid_depth, ask_depth);
    let mut h = inst.header(WireKind::Quote, recv_ns);
    h.set_depth(shape.depth).set_flags(shape.flags);

    *out = WireRecord::new_quote(h, quote);
    Ok(())
}
