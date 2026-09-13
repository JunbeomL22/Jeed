//! `@depth<N>` and `/fapi/v1/depth` — a Binance USD-M book, whole.
//!
//! Two envelopes, one decoder:
//!
//! ```json
//! {"lastUpdateId":1027024,"E":1606292218213,"T":1606292218208,
//!  "bids":[["4.00","431.00"]],"asks":[["4.01","12.00"]]}
//!
//! {"e":"depthUpdate","E":1571889248277,"T":1571889248276,"s":"BTCUSDT",
//!  "U":390497796,"u":390497878,"pu":390497794,
//!  "b":[["7403.89","0.002"]],"a":[["7405.96","3.340"]]}
//! ```
//!
//! The first is the REST response; the second is what `@depth10@100ms`
//! actually sends — USD-M dresses its partial-book stream in the `depthUpdate`
//! envelope even though the payload is a complete book, which spot does not
//! do. One decoder covers both because the collision is harmless: `l`
//! (`lastUpdateId`) and `u` both answer *the book is current as of*, `T`/`E`
//! both answer *when*, and `bids`/`b` and `asks`/`a` share a first byte by
//! construction.
//!
//! ## A partial book is a snapshot, not a delta
//!
//! `@depth10` frames say `depthUpdate` and carry `U`/`u`/`pu`, so they look
//! like the diff stream. They are not: each one is the top ten levels as they
//! now stand, replacing what came before, which is exactly
//! [`WireKind::Quote`]. Routing them to [`delta`](crate::binance::futures::delta) instead would
//! tell the consumer to *apply* a full book to its own — every level doubled
//! or, worse, every price outside the top ten left untouched forever.
//!
//! The choice is the caller's, because only the caller knows which stream it
//! subscribed to. Nothing in the frame distinguishes a ten-level partial book
//! from a ten-level diff.
//!
//! [`WireKind::Quote`]: jeed_wire::WireKind::Quote

use crate::binance::seen;
use crate::book::BookShape;
use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{next_key, parse_string_bytes, parse_u64, quote_levels, skip_value};
use crate::mask::require;
use crate::millis_to_nanos;
use jeed_wire::{QuotePayload, UnixNano, WireKind, WireRecord, quote_ext};

/// Every field the record needs.
const REQUIRED: u8 = seen::BIDS | seen::ASKS | seen::SEQUENCE;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] =
    &[(seen::BIDS, "bids"), (seen::ASKS, "asks"), (seen::SEQUENCE, "lastUpdateId")];

/// Decodes a USD-M depth snapshot — REST or partial-book stream — into `out`.
///
/// `out` is left untouched on error.
pub fn decode(
    inst: &Instrument,
    payload: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut quote = QuotePayload::default();
    let mut sequence: u64 = 0;
    let mut event_ms: Option<u64> = None;
    let mut txn_ms: Option<u64> = None;
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
            // `lastUpdateId` (REST) and `u` (stream) are the same answer.
            b'l' | b'u' => {
                let (v, next) = parse_u64(payload, pos);
                sequence = v;
                have |= seen::SEQUENCE;
                pos = next;
            }
            b'T' => {
                let (v, next) = parse_u64(payload, pos);
                txn_ms = Some(v);
                pos = next;
            }
            b'E' => {
                let (v, next) = parse_u64(payload, pos);
                event_ms = Some(v);
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
            b's' => {
                let (s, next) = parse_string_bytes(payload, pos);
                inst.check_symbol(s)?;
                pos = next;
            }
            // `U` and `pu` bound a diff and mean nothing for a whole book.
            _ => pos = skip_value(payload, pos),
        }
    }
    require(have, REQUIRED, KEYS)?;

    quote.with_quote_ext(quote_ext::SEQUENCE, sequence);

    let shape = BookShape::new(bid_depth, ask_depth);
    let mut h = inst.header(WireKind::Quote, recv_ns);
    h.set_depth(shape.depth).set_flags(shape.flags);
    // `T` says when the book changed, `E` when the message was pushed. REST
    // sends both; a very old REST response sends neither.
    if let Some(ms) = txn_ms.or(event_ms) {
        h.set_venue_time(millis_to_nanos(ms));
    }

    *out = WireRecord::new_quote(h, quote);
    Ok(())
}
