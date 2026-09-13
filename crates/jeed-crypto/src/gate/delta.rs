//! `spot.order_book_update` — a Gate book diff.
//!
//! ```json
//! {"time":1606294781,"time_ms":1606294781236,
//!  "channel":"spot.order_book_update","event":"update",
//!  "result":{"t":1606294781123,"e":"depthUpdate","E":1606294781,"s":"BTC_USDT",
//!            "U":48776301,"u":48776306,
//!            "b":[["19137.74","0.0001"],["19088.37","0"]],
//!            "a":[["19137.75","0.6135"]]}}
//! ```
//!
//! | key | |
//! |---|---|
//! | `result` | the message; everything below is inside it |
//! | `U` `u` | first and last update id this message covers |
//! | `b` `a` | `[[price, amount], …]`; amount `"0"` deletes the price |
//! | `t` | when the book changed (ms) → `venue_ns` |
//! | `s` | symbol — checked, then dropped |
//! | `e` `E` | Binance's event name and time, echoed — not read |
//! | `time` `time_ms` | when the frame was pushed — not read |
//!
//! ## Three clocks, and the one that is carried
//!
//! `time` and `time_ms` are when Gate sent the frame; `E` is a second-grained
//! copy of the same thing; `t` is when the book actually changed. `venue_ns`
//! is meant to be comparable with another venue's record of the same market
//! move, so it is `t` — the same choice
//! [`upbit::trade`](crate::upbit::trade) makes between `trade_timestamp` and
//! `timestamp`.
//!
//! ## No named predecessor, so the slot stays empty
//!
//! Gate sends `U` and `u` and nothing like Binance USD-M's `pu`, so
//! [`prev_final_update_id`](jeed_wire::SnapshotDeltaPayload::prev_final_update_id)
//! is left absent rather than filled with `U - 1`. The consumer chains on
//! `U`/`u`, which is what the venue gives it; inventing the other field would
//! be stating a fact Gate did not send (`CLAUDE.md`).

use crate::delta::Sides;
use crate::error::CryptoError;
use crate::gate::seen;
use crate::instrument::Instrument;
use crate::json::{next_field, object_at, parse_scalar_bytes, parse_scalar_u64, skip_value};
use crate::mask::require;
use crate::time::millis_to_nanos;
use jeed_wire::{SnapshotDeltaPayload, UnixNano, WireKind, WireRecord};

/// Every field the record needs.
const REQUIRED: u8 = seen::BIDS | seen::ASKS | seen::FIRST | seen::FINAL;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] =
    &[(seen::BIDS, "b"), (seen::ASKS, "a"), (seen::FIRST, "U"), (seen::FINAL, "u")];

/// Decodes a `spot.order_book_update` frame into `out`.
///
/// `out` is left untouched on error.
pub fn decode(
    inst: &Instrument,
    payload: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut result: Option<&[u8]> = None;

    let mut pos = 0;
    while pos < payload.len() {
        let (key, next) = next_field(payload, pos);
        pos = next;
        if key.is_empty() {
            break;
        }
        if key == b"result"
            && let Some((inner, past)) = object_at(payload, pos)
        {
            result = Some(inner);
            pos = past;
            continue;
        }
        pos = skip_value(payload, pos);
    }

    let Some(result) = result else {
        return Err(CryptoError::Missing { key: "result" });
    };
    body(inst, result, recv_ns, out)
}

/// Fills a [`WireKind::SnapshotDelta`] from the `result` object.
fn body(
    inst: &Instrument,
    data: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut d = SnapshotDeltaPayload::default();
    let mut sides = Sides::new();
    let mut book_ms: Option<u64> = None;
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
            b"U" => {
                let (v, next) = parse_scalar_u64(data, pos);
                d.first_update_id = v;
                have |= seen::FIRST;
                pos = next;
            }
            b"u" => {
                let (v, next) = parse_scalar_u64(data, pos);
                d.final_update_id = v;
                have |= seen::FINAL;
                pos = next;
            }
            b"t" => {
                let (v, next) = parse_scalar_u64(data, pos);
                book_ms = Some(v);
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
    // a delta has neither a book nor sides of equal length.
    let mut h = inst.header(WireKind::SnapshotDelta, recv_ns);
    if let Some(ms) = book_ms {
        h.set_venue_time(millis_to_nanos(ms));
    }

    *out = WireRecord::new_snapshot_delta(h, d);
    Ok(())
}
