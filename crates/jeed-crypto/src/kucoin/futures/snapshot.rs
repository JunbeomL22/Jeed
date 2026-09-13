//! `/api/v1/level2/snapshot` — a whole KuCoin futures book, from the REST
//! body.
//!
//! ```json
//! {"code":"200000","data":{"symbol":"XBTUSDTM","sequence":1709400450243,
//!                          "asks":[["90631.2",2],["90632.0",7]],
//!                          "bids":[["90630.8",5]]}}
//! ```
//!
//! | key | |
//! |---|---|
//! | `sequence` | the book's sequence → `quote_ext` [`SEQUENCE`] |
//! | `asks` `bids` | `[[price, size], …]` — price quoted, **size bare** |
//! | `symbol` | checked when present |
//!
//! ## Mixed spellings in one level
//!
//! `["90631.2",2]` is a quoted price beside a bare integer, which no other
//! venue in this crate does. Nothing special happens for it:
//! [`crate::json::parse_scalar_bytes`] hands both to the
//! extractor as the same digits, which is the whole reason that function
//! exists.
//!
//! ## No clock is read
//!
//! The body carries no timestamp this port has seen a verified example of, so
//! `venue_ns` is left unset rather than filled from a field whose units would
//! be a guess — and on this venue the guess is between milliseconds and
//! nanoseconds, which is a factor of a million (see
//! [`kucoin`](crate::kucoin)). `recv_ns` still says when the handler had it.
//!
//! [`SEQUENCE`]: jeed_wire::quote_ext::SEQUENCE

use crate::book::BookShape;
use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{next_field, object_at, parse_scalar_bytes, parse_scalar_u64, quote_levels, skip_value};
use crate::kucoin::seen;
use crate::mask::require;
use jeed_wire::{QuotePayload, UnixNano, WireKind, WireRecord, quote_ext};

/// Every field the record needs.
const REQUIRED: u8 = seen::BIDS | seen::ASKS;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[(seen::BIDS, "bids"), (seen::ASKS, "asks")];

/// Decodes a REST level2 snapshot body into `out`.
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
            b"symbol" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                inst.check_symbol(v)?;
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

    *out = WireRecord::new_quote(h, quote);
    Ok(())
}
