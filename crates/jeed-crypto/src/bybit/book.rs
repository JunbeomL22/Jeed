//! `orderbook.{depth}.{symbol}` — a Bybit book, whole or in pieces.
//!
//! ```json
//! {"topic":"orderbook.50.BTCUSDT","type":"snapshot","ts":1672304484978,
//!  "data":{"s":"BTCUSDT",
//!          "b":[["16493.50","0.006"],["16493.00","0.100"]],
//!          "a":[["16611.00","0.029"],["16612.00","0.213"]],
//!          "u":18521288,"seq":7961638724},
//!  "cts":1672304484976}
//! ```
//!
//! | key | |
//! |---|---|
//! | `type` | `snapshot` or `delta` |
//! | `cts` | matching-engine time (ms) → `venue_ns` |
//! | `ts` | when Bybit's system built the message — fallback only |
//! | `data.s` | symbol — checked, then dropped |
//! | `data.b` `data.a` | `[["price","size"], …]`; **a size of `0` deletes the price** |
//! | `data.u` | orderbook update id, `+1` per message |
//! | `data.seq` | cross-stream sequence — see [`bybit`](crate::bybit) |
//!
//! ## `u == 1` is a snapshot whatever `type` says
//!
//! Bybit restarts the orderbook service from time to time and resets `u` to 1.
//! The frame that carries it may still be typed `delta`, and applying it as
//! one would merge a fresh book into a stale one. So the routing is:
//!
//! | | record |
//! |---|---|
//! | `type: "snapshot"` | [`WireKind::Quote`] |
//! | `type: "delta"`, `u == 1` | [`WireKind::Quote`] |
//! | `type: "delta"`, `u > 1` | [`WireKind::SnapshotDelta`] |
//! | any other `type` | [`CryptoError::Unexpected`] — nothing published |
//!
//! Deciding that is the one thing here the consumer could not do for itself
//! without knowing Bybit's restart convention, which is precisely the kind of
//! venue-specific rule the handler exists to resolve: the wire carries the
//! conclusion — *this is a whole book* — and not the evidence
//! (`documents/feed_handler.md` §8). It also means `u` has to be read before
//! the payload can be chosen, which is why the `data` object is scanned for it
//! first and walked properly afterwards. That scan counts brackets over the
//! level arrays rather than parsing them, so it costs a pass over the bytes
//! and no decimal work.
//!
//! ## The chain
//!
//! One `u` per message and no `pu`-equivalent, so `first_update_id` and
//! `final_update_id` are both `u` and
//! [`prev_final_update_id`](jeed_wire::SnapshotDeltaPayload::prev_final_update_id)
//! stays absent — the consumer chains on `u + 1`, which is Bybit's own rule.
//! Leaving it absent rather than zero is what lets the consumer tell "this
//! venue does not name its predecessor" from "its predecessor was message
//! zero".
//!
//! [`WireKind::Quote`]: jeed_wire::WireKind::Quote
//! [`WireKind::SnapshotDelta`]: jeed_wire::WireKind::SnapshotDelta

use crate::book::BookShape;
use crate::bybit::seen;
use crate::delta::Sides;
use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{
    next_field, object_at, parse_scalar_bytes, parse_scalar_u64, quote_levels, skip_value,
};
use crate::mask::require;
use crate::millis_to_nanos;
use jeed_wire::{
    QuotePayload, SnapshotDeltaPayload, UnixNano, WireKind, WireRecord, quote_ext,
};

/// Every field the record needs, either way.
const REQUIRED: u8 = seen::BIDS | seen::ASKS | seen::UPDATE_ID;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[(seen::BIDS, "b"), (seen::ASKS, "a"), (seen::UPDATE_ID, "u")];

/// The `u` a restart resets to, and therefore the one that means *rebuild*.
const REBUILD_UPDATE_ID: u64 = 1;

/// Decodes an `orderbook` frame into `out`.
///
/// `out` is left untouched on error.
pub fn decode(
    inst: &Instrument,
    payload: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut is_snapshot: Option<bool> = None;
    let mut data: Option<&[u8]> = None;
    let mut system_ms: Option<u64> = None;
    let mut engine_ms: Option<u64> = None;

    let mut pos = 0;
    while pos < payload.len() {
        let (key, next) = next_field(payload, pos);
        pos = next;
        if key.is_empty() {
            break;
        }

        match key {
            b"type" => {
                let (v, next) = parse_scalar_bytes(payload, pos);
                is_snapshot = Some(match v {
                    b"snapshot" => true,
                    b"delta" => false,
                    _ => return Err(CryptoError::Unexpected { key: "type" }),
                });
                pos = next;
            }
            b"data" => {
                let Some((inner, past)) = object_at(payload, pos) else {
                    return Err(CryptoError::Missing { key: "data" });
                };
                data = Some(inner);
                pos = past;
            }
            b"cts" => {
                let (v, next) = parse_scalar_u64(payload, pos);
                engine_ms = Some(v);
                pos = next;
            }
            b"ts" => {
                let (v, next) = parse_scalar_u64(payload, pos);
                system_ms = Some(v);
                pos = next;
            }
            _ => pos = skip_value(payload, pos),
        }
    }

    let Some(is_snapshot) = is_snapshot else {
        return Err(CryptoError::Missing { key: "type" });
    };
    let Some(data) = data else {
        return Err(CryptoError::Missing { key: "data" });
    };

    // `cts` is when the matching engine produced the book; `ts` is when the
    // system got round to serialising it. Spot does not always send `cts`.
    let venue_ms = engine_ms.or(system_ms);

    let update_id = peek_update_id(data).ok_or(CryptoError::Missing { key: "u" })?;
    if is_snapshot || update_id == REBUILD_UPDATE_ID {
        snapshot(inst, data, recv_ns, venue_ms, out)
    } else {
        delta(inst, data, recv_ns, venue_ms, out)
    }
}

/// Reads `u` out of the `data` object without parsing anything else.
///
/// The level arrays are stepped over by counting brackets, so this is a pass
/// over the bytes and not a second decode.
fn peek_update_id(data: &[u8]) -> Option<u64> {
    let mut pos = 0;
    while pos < data.len() {
        let (key, next) = next_field(data, pos);
        pos = next;
        if key.is_empty() {
            break;
        }
        if key == b"u" {
            return Some(parse_scalar_u64(data, pos).0);
        }
        pos = skip_value(data, pos);
    }
    None
}

/// Fills a [`WireKind::Quote`] from the `data` object.
fn snapshot(
    inst: &Instrument,
    data: &[u8],
    recv_ns: UnixNano,
    venue_ms: Option<u64>,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut quote = QuotePayload::default();
    let mut update_id: u64 = 0;
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
            b"b" => {
                let (d, next) = quote_levels(inst, data, pos, &mut quote.bid)?;
                bid_depth = d;
                have |= seen::BIDS;
                pos = next;
            }
            b"a" => {
                let (d, next) = quote_levels(inst, data, pos, &mut quote.ask)?;
                ask_depth = d;
                have |= seen::ASKS;
                pos = next;
            }
            b"u" => {
                let (v, next) = parse_scalar_u64(data, pos);
                update_id = v;
                have |= seen::UPDATE_ID;
                pos = next;
            }
            b"s" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                inst.check_symbol(v)?;
                pos = next;
            }
            _ => pos = skip_value(data, pos),
        }
    }
    require(have, REQUIRED, KEYS)?;

    quote.with_quote_ext(quote_ext::SEQUENCE, update_id);

    let shape = BookShape::new(bid_depth, ask_depth);
    let mut h = inst.header(WireKind::Quote, recv_ns);
    h.set_depth(shape.depth).set_flags(shape.flags);
    if let Some(ms) = venue_ms {
        h.set_venue_time(millis_to_nanos(ms));
    }

    *out = WireRecord::new_quote(h, quote);
    Ok(())
}

/// Fills a [`WireKind::SnapshotDelta`] from the `data` object.
fn delta(
    inst: &Instrument,
    data: &[u8],
    recv_ns: UnixNano,
    venue_ms: Option<u64>,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut d = SnapshotDeltaPayload::default();
    let mut sides = Sides::new();
    let mut have = 0u8;

    let mut pos = 0;
    while pos < data.len() {
        let (key, next) = next_field(data, pos);
        pos = next;
        if key.is_empty() {
            break;
        }

        match key {
            b"b" => {
                pos = sides.read(inst, data, pos, &mut d.levels, true)?;
                have |= seen::BIDS;
            }
            b"a" => {
                pos = sides.read(inst, data, pos, &mut d.levels, false)?;
                have |= seen::ASKS;
            }
            b"u" => {
                let (v, next) = parse_scalar_u64(data, pos);
                // One message, one update id: both ends of what it covers.
                d.first_update_id = v;
                d.final_update_id = v;
                have |= seen::UPDATE_ID;
                pos = next;
            }
            b"s" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                inst.check_symbol(v)?;
                pos = next;
            }
            _ => pos = skip_value(data, pos),
        }
    }
    require(have, REQUIRED, KEYS)?;

    sides.finish(&mut d);

    // Depth stays zero: the header's `depth` is a book's levels per side, and
    // a delta has neither a book nor sides of equal length. The counts are in
    // the payload.
    let mut h = inst.header(WireKind::SnapshotDelta, recv_ns);
    if let Some(ms) = venue_ms {
        h.set_venue_time(millis_to_nanos(ms));
    }

    *out = WireRecord::new_snapshot_delta(h, d);
    Ok(())
}
