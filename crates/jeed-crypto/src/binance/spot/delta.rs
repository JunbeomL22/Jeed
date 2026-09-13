//! `@depth` — Binance spot incremental book updates.
//!
//! ```json
//! {"e":"depthUpdate","E":1672515782136,"s":"BNBBTC","U":157,"u":160,
//!  "b":[["0.0024","10"]],"a":[["0.0026","0"]]}
//! ```
//!
//! | key | |
//! |---|---|
//! | `E` | event time (ms) → `venue_ns`. Spot has no `T` |
//! | `U` | first update id this message covers |
//! | `u` | last update id this message covers |
//! | `b` `a` | changed levels; **a quantity of `0` deletes the price** |
//!
//! ## The consumer's obligations, and why they are the consumer's
//!
//! Binance's own rule: apply a message only when `U` is one past the last `u`
//! applied, and resynchronise from `/api/v3/depth` otherwise. Spot sends no
//! `pu`, so the chain is `U`/`u` alone and
//! [`prev_final_update_id`] is left absent rather than
//! guessed at.
//!
//! None of that is done here. This crate is stateless in the same sense the
//! rest of the feed handler is (`documents/feed_handler.md` §1): it holds no
//! book, so it has no book to keep in sync, and a gap detector living here
//! would be a second opinion the consumer could not act on anyway — the
//! consumer is the one that has to buffer and replay. What the handler owes
//! is the chain, intact, which is what this carries.
//!
//! ## Overflow is a dropped frame, not a shorter one
//!
//! A message with more changes than
//! [`WIRE_MAX_DELTA_LEVELS`](jeed_wire::WIRE_MAX_DELTA_LEVELS) fails with
//! [`CryptoError::DeltaOverflow`] and nothing is published. The consumer then
//! meets a hole in the update-id chain and resynchronises — the machinery it
//! already needs for a ring drop. Publishing the first thirty-two changes of
//! forty would instead be accepted as complete and leave the book wrong for
//! as long as the process runs.
//!
//! [`prev_final_update_id`]: jeed_wire::SnapshotDeltaPayload::prev_final_update_id

use crate::binance::seen;
use crate::delta::Sides;
use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{next_key, parse_string_bytes, parse_u64, skip_value};
use crate::mask::require;
use crate::millis_to_nanos;
use jeed_wire::{SnapshotDeltaPayload, UnixNano, WireKind, WireRecord};

/// Every field the record needs.
const REQUIRED: u8 = seen::BIDS | seen::ASKS | seen::SEQUENCE;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[(seen::BIDS, "b"), (seen::ASKS, "a"), (seen::SEQUENCE, "u")];

/// Decodes a spot `@depth` frame into `out`.
///
/// `out` is left untouched on error — which for this kind matters more than
/// for any other, since a partly-applied delta is not a stale book but a
/// wrong one.
pub fn decode(
    inst: &Instrument,
    payload: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut d = SnapshotDeltaPayload::default();
    let mut sides = Sides::new();
    let mut event_ms: Option<u64> = None;
    let mut have = 0u8;

    let mut pos = 0;
    while pos < payload.len() {
        let (key, next) = next_key(payload, pos);
        pos = next;
        if key == 0 {
            break;
        }

        match key {
            b'E' => {
                let (v, next) = parse_u64(payload, pos);
                event_ms = Some(v);
                pos = next;
            }
            b'U' => {
                let (v, next) = parse_u64(payload, pos);
                d.first_update_id = v;
                pos = next;
            }
            b'u' => {
                let (v, next) = parse_u64(payload, pos);
                d.final_update_id = v;
                have |= seen::SEQUENCE;
                pos = next;
            }
            b'b' => {
                pos = sides.read(inst, payload, pos, &mut d.levels, true)?;
                have |= seen::BIDS;
            }
            b'a' => {
                pos = sides.read(inst, payload, pos, &mut d.levels, false)?;
                have |= seen::ASKS;
            }
            b's' => {
                let (s, next) = parse_string_bytes(payload, pos);
                inst.check_symbol(s)?;
                pos = next;
            }
            _ => pos = skip_value(payload, pos),
        }
    }
    require(have, REQUIRED, KEYS)?;
    sides.finish(&mut d);

    // Depth stays zero: the header's `depth` is a book's levels per side, and
    // a delta has neither a book nor sides of equal length. The counts are in
    // the payload.
    let mut h = inst.header(WireKind::SnapshotDelta, recv_ns);
    if let Some(ms) = event_ms {
        h.set_venue_time(millis_to_nanos(ms));
    }

    *out = WireRecord::new_snapshot_delta(h, d);
    Ok(())
}
