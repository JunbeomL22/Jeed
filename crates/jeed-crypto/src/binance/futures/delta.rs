//! `@depth` — Binance USD-M incremental book updates.
//!
//! ```json
//! {"e":"depthUpdate","E":1571889248277,"T":1571889248276,"s":"BTCUSDT",
//!  "U":390497796,"u":390497878,"pu":390497794,
//!  "b":[["7403.89","0.002"]],"a":[["7405.96","0"]]}
//! ```
//!
//! Spot's [`delta`](crate::binance::spot::delta) plus two fields:
//!
//! - **`T`** — when the book changed, in preference to `E`, which is when the
//!   message was pushed.
//! - **`pu`** — the `u` of the message this one must follow. It makes the
//!   chain checkable without the `U == last u + 1` assumption, which USD-M
//!   explicitly does not guarantee across a reconnect. It rides through as
//!   [`prev_final_update_id`], flagged, because spot has no such field and a
//!   zero must not be mistaken for "follows message zero".
//!
//! Everything else — zero quantity deletes, overflow drops the frame, gap
//! detection is the consumer's — is as spot; see that module.
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

/// Every field the record needs. `pu` is not among them: it is USD-M's
/// convenience, and a frame without it still chains on `U`/`u`.
const REQUIRED: u8 = seen::BIDS | seen::ASKS | seen::SEQUENCE;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[(seen::BIDS, "b"), (seen::ASKS, "a"), (seen::SEQUENCE, "u")];

/// Decodes a USD-M `@depth` frame into `out`.
///
/// `out` is left untouched on error.
pub fn decode(
    inst: &Instrument,
    payload: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut d = SnapshotDeltaPayload::default();
    let mut sides = Sides::new();
    let mut event_ms: Option<u64> = None;
    let mut txn_ms: Option<u64> = None;
    let mut prev_final: Option<u64> = None;
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
            b'T' => {
                let (v, next) = parse_u64(payload, pos);
                txn_ms = Some(v);
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
            // `pu` — the only key in this message starting with `p`.
            b'p' => {
                let (v, next) = parse_u64(payload, pos);
                prev_final = Some(v);
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
    if let Some(v) = prev_final {
        d.with_prev_final_update_id(v);
    }

    let mut h = inst.header(WireKind::SnapshotDelta, recv_ns);
    if let Some(ms) = txn_ms.or(event_ms) {
        h.set_venue_time(millis_to_nanos(ms));
    }

    *out = WireRecord::new_snapshot_delta(h, d);
    Ok(())
}
