//! `books` · `books1` · `books5` · `books15` — a Bitget book, whole or in
//! pieces, on one subscription.
//!
//! ```json
//! {"action":"update",
//!  "arg":{"instType":"USDT-FUTURES","channel":"books","instId":"BTCUSDT"},
//!  "data":[{"asks":[["27000.5","8.760"],["27003.0","0"]],
//!           "bids":[["27000.0","2.710"]],
//!           "checksum":-855196043,"seq":"456","pseq":"455",
//!           "ts":"1695716059616"}]}
//! ```
//!
//! | key | |
//! |---|---|
//! | `action` | `snapshot` or `update` |
//! | `seq` | this message's sequence number (quoted) |
//! | `pseq` | the `seq` this message must follow (quoted) |
//! | `ts` | when the book changed (ms, quoted) → `venue_ns` |
//! | `asks` `bids` | `[[price, size], …]`, best first; size `"0"` deletes |
//! | `checksum` | CRC32 — see [`bitget`](crate::bitget) |
//!
//! ## One subscription, two record kinds
//!
//! | `action` | record |
//! |---|---|
//! | `"snapshot"` | [`WireKind::Quote`] |
//! | `"update"` | [`WireKind::SnapshotDelta`] |
//! | absent | [`CryptoError::Missing`] |
//! | anything else | [`CryptoError::Unexpected`] — nothing published |
//!
//! Absent is an error here where OKX tolerates it, because OKX's `books5`
//! pushes have no `action` and Bitget's do: every Bitget book message, on
//! every one of the four channels, says which it is. A message that stopped
//! saying is a protocol change, not a shallow-book push.
//!
//! The shallow channels (`books1`, `books5`, `books15`) send `"snapshot"`
//! every time and so produce a [`WireKind::Quote`] every time, which is what
//! they are: a whole book, five levels deep. Only `books` sends `"update"`.
//!
//! ## The sequence chain, mapped
//!
//! `pseq → seq` is OKX's `prevSeqId → seqId` with different letters, so it
//! lands the same way: one message covers one update, `first_update_id` and
//! `final_update_id` are both `seq`, and `pseq` rides in
//! [`prev_final_update_id`](jeed_wire::SnapshotDeltaPayload::prev_final_update_id).
//!
//! (`fractal-engine` reverses this chain exactly as its OKX decoder does —
//! `first_update_id = seq`, `final_update_id = pseq`. The same mistake twice
//! is worth naming once more: both fields end up populated with plausible
//! numbers, so nothing downstream can notice.)
//!
//! [`WireKind::Quote`]: jeed_wire::WireKind::Quote
//! [`WireKind::SnapshotDelta`]: jeed_wire::WireKind::SnapshotDelta

use crate::bitget::seen;
use crate::book::BookShape;
use crate::delta::Sides;
use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{
    next_field, object_at, objects_at, parse_scalar_bytes, parse_scalar_u64, quote_levels, skip_value,
};
use crate::mask::require;
use crate::time::millis_to_nanos;
use jeed_wire::{QuotePayload, SnapshotDeltaPayload, UnixNano, WireKind, WireRecord, quote_ext};

/// Every field a whole book needs.
const SNAPSHOT_REQUIRED: u8 = seen::BIDS | seen::ASKS | seen::SEQUENCE;

/// Every field a diff needs. `pseq` is among them: without it the consumer
/// cannot tell a delta it may apply from one that would corrupt its book.
const DELTA_REQUIRED: u8 = SNAPSHOT_REQUIRED | seen::PREV_SEQUENCE;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[
    (seen::BIDS, "bids"),
    (seen::ASKS, "asks"),
    (seen::SEQUENCE, "seq"),
    (seen::PREV_SEQUENCE, "pseq"),
];

/// What `action` said.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    /// `"snapshot"`.
    Snapshot,
    /// `"update"`.
    Update,
}

/// Decodes a `books` family frame into `out`.
///
/// `out` is left untouched on error.
pub fn decode(
    inst: &Instrument,
    payload: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut action: Option<Action> = None;
    let mut data: Option<&[u8]> = None;

    // The envelope first, whole: `action` may follow `data`, and both have to
    // be in hand before either can be used.
    let mut pos = 0;
    while pos < payload.len() {
        let (key, next) = next_field(payload, pos);
        pos = next;
        if key.is_empty() {
            break;
        }

        match key {
            b"action" => {
                let (v, next) = parse_scalar_bytes(payload, pos);
                action = Some(match v {
                    b"snapshot" => Action::Snapshot,
                    b"update" => Action::Update,
                    _ => return Err(CryptoError::Unexpected { key: "action" }),
                });
                pos = next;
            }
            b"data" => {
                data = objects_at(payload, pos).and_then(|mut o| o.next());
                pos = skip_value(payload, pos);
            }
            b"arg" => {
                if let Some((arg, past)) = object_at(payload, pos) {
                    check_inst_id(inst, arg)?;
                    pos = past;
                } else {
                    pos = skip_value(payload, pos);
                }
            }
            _ => pos = skip_value(payload, pos),
        }
    }

    let Some(action) = action else {
        return Err(CryptoError::Missing { key: "action" });
    };
    let Some(data) = data else {
        return Err(CryptoError::Missing { key: "data" });
    };

    match action {
        Action::Snapshot => snapshot(inst, data, recv_ns, out),
        Action::Update => delta(inst, data, recv_ns, out),
    }
}

/// Checks `arg.instId` if the envelope names one.
///
/// The `data` objects do not carry the symbol at all, so this is the only
/// place a mis-wired subscription can be caught — see `CLAUDE.md` on why it is
/// worth catching.
pub(crate) fn check_inst_id(inst: &Instrument, arg: &[u8]) -> Result<(), CryptoError> {
    let mut pos = 0;
    while pos < arg.len() {
        let (key, next) = next_field(arg, pos);
        pos = next;
        if key.is_empty() {
            break;
        }
        if key == b"instId" {
            let (v, _) = parse_scalar_bytes(arg, pos);
            return inst.check_symbol(v);
        }
        pos = skip_value(arg, pos);
    }
    Ok(())
}

/// Fills a [`WireKind::Quote`] from one `data` object.
fn snapshot(
    inst: &Instrument,
    data: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut quote = QuotePayload::default();
    let mut seq: u64 = 0;
    let mut ts_ms: Option<u64> = None;
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
            b"seq" => {
                let (v, next) = parse_scalar_u64(data, pos);
                seq = v;
                have |= seen::SEQUENCE;
                pos = next;
            }
            b"ts" => {
                let (v, next) = parse_scalar_u64(data, pos);
                ts_ms = Some(v);
                pos = next;
            }
            _ => pos = skip_value(data, pos),
        }
    }
    require(have, SNAPSHOT_REQUIRED, KEYS)?;

    quote.with_quote_ext(quote_ext::SEQUENCE, seq);

    let shape = BookShape::new(bid_depth, ask_depth);
    let mut h = inst.header(WireKind::Quote, recv_ns);
    h.set_depth(shape.depth).set_flags(shape.flags);
    if let Some(ms) = ts_ms {
        h.set_venue_time(millis_to_nanos(ms));
    }

    *out = WireRecord::new_quote(h, quote);
    Ok(())
}

/// Fills a [`WireKind::SnapshotDelta`] from one `data` object.
fn delta(
    inst: &Instrument,
    data: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut d = SnapshotDeltaPayload::default();
    let mut sides = Sides::new();
    let mut pseq: u64 = 0;
    let mut ts_ms: Option<u64> = None;
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
                pos = sides.read(inst, data, pos, &mut d.levels, true)?;
                have |= seen::BIDS;
            }
            b"asks" => {
                pos = sides.read(inst, data, pos, &mut d.levels, false)?;
                have |= seen::ASKS;
            }
            b"seq" => {
                let (v, next) = parse_scalar_u64(data, pos);
                // One message, one sequence number: both ends of the range
                // this record covers.
                d.first_update_id = v;
                d.final_update_id = v;
                have |= seen::SEQUENCE;
                pos = next;
            }
            b"pseq" => {
                let (v, next) = parse_scalar_u64(data, pos);
                pseq = v;
                have |= seen::PREV_SEQUENCE;
                pos = next;
            }
            b"ts" => {
                let (v, next) = parse_scalar_u64(data, pos);
                ts_ms = Some(v);
                pos = next;
            }
            _ => pos = skip_value(data, pos),
        }
    }
    require(have, DELTA_REQUIRED, KEYS)?;

    sides.finish(&mut d);
    d.with_prev_final_update_id(pseq);

    // Depth stays zero: the header's `depth` is a book's levels per side, and
    // a delta has neither a book nor sides of equal length.
    let mut h = inst.header(WireKind::SnapshotDelta, recv_ns);
    if let Some(ms) = ts_ms {
        h.set_venue_time(millis_to_nanos(ms));
    }

    *out = WireRecord::new_snapshot_delta(h, d);
    Ok(())
}
