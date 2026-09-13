//! `/market/level2` — a KuCoin spot book diff.
//!
//! ```json
//! {"type":"message","topic":"/market/level2:BTC-USDT","subject":"trade.l2update",
//!  "data":{"sequenceStart":1545896669105,"sequenceEnd":1545896669106,
//!          "symbol":"BTC-USDT",
//!          "changes":{"asks":[["6","1","1545896669105"]],
//!                     "bids":[["4","1","1545896669106"]]}}}
//! ```
//!
//! | key | |
//! |---|---|
//! | `sequenceStart` `sequenceEnd` | the range of updates this message covers |
//! | `changes.bids` `changes.asks` | `[[price, size, sequence], …]`; size `"0"` deletes |
//! | `symbol` | checked, then dropped |
//! | `topic` | checked too, when `symbol` is absent |
//!
//! ## A range, so it maps straight onto the wire's range
//!
//! `sequenceStart`…`sequenceEnd` is Binance's `U`…`u` under another name: a
//! message can cover several updates, and the consumer's rule is that the next
//! `sequenceStart` must not skip past the last `sequenceEnd`. Both ends are
//! carried; there is no named predecessor, so
//! [`prev_final_update_id`](jeed_wire::SnapshotDeltaPayload::prev_final_update_id)
//! stays absent.
//!
//! ## The third element of a level is a sequence, and it is skipped
//!
//! `["6","1","1545896669105"]` is price, size, and the sequence at which *that
//! level* changed — KuCoin's own way of letting a client apply changes out of
//! order against a snapshot. The wire has no per-level slot for it, and a
//! consumer applying a whole record at a time does not need one: the record's
//! range already says where it belongs.
//!
//! ## There is no clock in this message
//!
//! No `time`, no `ts`, nothing — so `venue_ns` is left unset rather than
//! filled from the frame's arrival. `recv_ns` is already the handler's
//! measurement of when it turned up, and copying it into the venue's slot
//! would claim the exchange said something it did not (`CLAUDE.md`).

use crate::delta::Sides;
use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{next_field, object_at, parse_scalar_bytes, parse_scalar_u64, skip_value};
use crate::kucoin::{check_topic, seen};
use crate::mask::require;
use jeed_wire::{SnapshotDeltaPayload, UnixNano, WireKind, WireRecord};

/// Every field the record needs.
const REQUIRED: u8 = seen::BIDS | seen::ASKS | seen::SEQUENCE | seen::SEQUENCE_END;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[
    (seen::BIDS, "changes.bids"),
    (seen::ASKS, "changes.asks"),
    (seen::SEQUENCE, "sequenceStart"),
    (seen::SEQUENCE_END, "sequenceEnd"),
];

/// Decodes a `/market/level2` frame into `out`.
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

        match key {
            b"data" => {
                if let Some((inner, past)) = object_at(payload, pos) {
                    data = Some(inner);
                    pos = past;
                } else {
                    pos = skip_value(payload, pos);
                }
            }
            b"topic" => {
                let (v, next) = parse_scalar_bytes(payload, pos);
                check_topic(inst, v)?;
                pos = next;
            }
            _ => pos = skip_value(payload, pos),
        }
    }

    let Some(data) = data else {
        return Err(CryptoError::Missing { key: "data" });
    };
    body(inst, data, recv_ns, out)
}

/// Fills a [`WireKind::SnapshotDelta`] from the `data` object.
fn body(
    inst: &Instrument,
    data: &[u8],
    recv_ns: UnixNano,
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
            b"changes" => {
                let Some((changes, past)) = object_at(data, pos) else {
                    return Err(CryptoError::Missing { key: "changes" });
                };
                have |= read_changes(inst, changes, &mut d, &mut sides)?;
                pos = past;
            }
            b"sequenceStart" => {
                let (v, next) = parse_scalar_u64(data, pos);
                d.first_update_id = v;
                have |= seen::SEQUENCE;
                pos = next;
            }
            b"sequenceEnd" => {
                let (v, next) = parse_scalar_u64(data, pos);
                d.final_update_id = v;
                have |= seen::SEQUENCE_END;
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

    sides.finish(&mut d);

    // Depth stays zero: the header's `depth` is a book's levels per side, and
    // a delta has neither a book nor sides of equal length. No `venue_ns`
    // either — this message carries no clock of its own.
    let h = inst.header(WireKind::SnapshotDelta, recv_ns);

    *out = WireRecord::new_snapshot_delta(h, d);
    Ok(())
}

/// Reads the `changes` object's two sides, returning the bits it filled.
fn read_changes(
    inst: &Instrument,
    changes: &[u8],
    d: &mut SnapshotDeltaPayload,
    sides: &mut Sides,
) -> Result<u8, CryptoError> {
    let mut have = 0u8;

    let mut pos = 0;
    while pos < changes.len() {
        let (key, next) = next_field(changes, pos);
        pos = next;
        if key.is_empty() {
            break;
        }

        match key {
            b"bids" => {
                pos = sides.read(inst, changes, pos, &mut d.levels, true)?;
                have |= seen::BIDS;
            }
            b"asks" => {
                pos = sides.read(inst, changes, pos, &mut d.levels, false)?;
                have |= seen::ASKS;
            }
            _ => pos = skip_value(changes, pos),
        }
    }

    Ok(have)
}
