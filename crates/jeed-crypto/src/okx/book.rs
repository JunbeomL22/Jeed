//! `books` · `books-l2-tbt` · `books50-l2-tbt` · `books5` — an OKX book,
//! whole or in pieces, on one subscription.
//!
//! ```json
//! {"arg":{"channel":"books","instId":"BTC-USDT"},"action":"snapshot",
//!  "data":[{"asks":[["64000.5","0.01","0","1"]],
//!           "bids":[["63999.5","0.02","0","1"]],
//!           "ts":"1706000000000","checksum":-855196043,
//!           "seqId":1234567890,"prevSeqId":1234567889}]}
//! ```
//!
//! | key | |
//! |---|---|
//! | `action` | `snapshot` or `update`; **absent on `books5`** |
//! | `seqId` | this message's sequence number |
//! | `prevSeqId` | the `seqId` this message must follow |
//! | `ts` | when the book changed (ms, quoted) → `venue_ns` |
//! | `asks` `bids` | `[[price, size, "0", order count], …]`, best first |
//! | `checksum` | CRC32 of the book after applying this — see [`okx`](crate::okx) |
//!
//! ## One subscription, two record kinds
//!
//! Every other book decoder in this crate belongs to one stream and produces
//! one kind. OKX's `books` channel sends a `snapshot` first and then `update`
//! frames forever, so this routes on `action`:
//!
//! | `action` | record |
//! |---|---|
//! | absent (`books5`) | [`WireKind::Quote`] |
//! | `"snapshot"` | [`WireKind::Quote`] |
//! | `"update"` | [`WireKind::SnapshotDelta`] |
//! | anything else | [`CryptoError::Unexpected`] — nothing published |
//!
//! There is no safe default for that last row, which is why it is an error
//! rather than a guess: reading a diff as a snapshot throws away every level
//! the message did not mention, and reading a snapshot as a diff leaves every
//! price outside it untouched forever. The caller learns which was produced
//! from `out.header.kind()`.
//!
//! ## The sequence chain, mapped
//!
//! OKX carries one sequence number per message and names its predecessor, so
//! the chain is `prevSeqId → seqId` where Binance's is `U…u`. A message
//! therefore covers exactly one update, and `first_update_id` and
//! `final_update_id` are both `seqId`; `prevSeqId` is what the consumer
//! actually checks and rides in
//! [`prev_final_update_id`](jeed_wire::SnapshotDeltaPayload::prev_final_update_id)
//! with [`delta_flags::PREV_FINAL_VALID`](jeed_wire::delta_flags::PREV_FINAL_VALID)
//! set — the same slot Binance USD-M's `pu` uses, because it answers the same
//! question.
//!
//! (`fractal-engine` put `seqId` in `first_update_id` and `prevSeqId` in
//! `final_update_id`, which reverses the chain. Worth naming, because both
//! fields are populated and plausible and nothing downstream would have
//! complained.)
//!
//! ## An empty `update` is a keep-alive, not a fault
//!
//! When nothing has changed for a while OKX sends `"asks":[],"bids":[]` with
//! the `seqId` it last sent, to show the connection is alive. That decodes to
//! a delta with no levels and a chain that does not advance, which is exactly
//! what happened.
//!
//! ## Order counts are not carried
//!
//! The fourth element of a level is the number of orders resting at that
//! price, and [`WireLevel`](jeed_wire::WireLevel) has a field for it. It is
//! dropped anyway, because [`WireDeltaLevel`](jeed_wire::WireDeltaLevel) does
//! not: a consumer that got counts from the snapshot and no counts from the
//! diffs would hold a book whose counts were right once and wrong from the
//! first update on. Absent everywhere beats correct for one message.
//!
//! [`WireKind::Quote`]: jeed_wire::WireKind::Quote
//! [`WireKind::SnapshotDelta`]: jeed_wire::WireKind::SnapshotDelta

use crate::book::BookShape;
use crate::delta::Sides;
use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{
    next_field, object_at, objects_at, parse_scalar_bytes, parse_scalar_u64, quote_levels, skip_value,
};
use crate::mask::require;
use crate::millis_to_nanos;
use crate::okx::seen;
use jeed_wire::{
    QuotePayload, SnapshotDeltaPayload, UnixNano, WireKind, WireRecord, quote_ext,
};

/// Every field a whole book needs.
const SNAPSHOT_REQUIRED: u8 = seen::BIDS | seen::ASKS | seen::SEQUENCE;

/// Every field a diff needs. `prevSeqId` is among them: without it the
/// consumer has no way to tell a delta it may apply from one that would
/// corrupt its book.
const DELTA_REQUIRED: u8 = SNAPSHOT_REQUIRED | seen::PREV_SEQUENCE;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[
    (seen::BIDS, "bids"),
    (seen::ASKS, "asks"),
    (seen::SEQUENCE, "seqId"),
    (seen::PREV_SEQUENCE, "prevSeqId"),
];

/// What `action` said.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    /// No `action` key — a `books5` push, which is always a whole book.
    Absent,
    /// `"snapshot"`.
    Snapshot,
    /// `"update"`.
    Update,
}

/// Decodes a `books` family frame into `out`.
///
/// `out` is left untouched on error — which for the `update` branch matters
/// more than anywhere else, since a partly-applied delta is not a stale book
/// but a wrong one.
pub fn decode(
    inst: &Instrument,
    payload: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut action = Action::Absent;
    let mut data: Option<&[u8]> = None;

    // The envelope first, whole: `action` may follow `data` and the two have
    // to be in hand together before either can be used.
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
                action = match v {
                    b"snapshot" => Action::Snapshot,
                    b"update" => Action::Update,
                    _ => return Err(CryptoError::Unexpected { key: "action" }),
                };
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

    let Some(data) = data else {
        return Err(CryptoError::Missing { key: "data" });
    };

    match action {
        Action::Absent | Action::Snapshot => snapshot(inst, data, recv_ns, out),
        Action::Update => delta(inst, data, recv_ns, out),
    }
}

/// Checks `arg.instId` if the envelope names one.
fn check_inst_id(inst: &Instrument, arg: &[u8]) -> Result<(), CryptoError> {
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
    let mut seq_id: u64 = 0;
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
            b"seqId" => {
                let (v, next) = parse_scalar_u64(data, pos);
                seq_id = v;
                have |= seen::SEQUENCE;
                pos = next;
            }
            b"ts" => {
                let (v, next) = parse_scalar_u64(data, pos);
                ts_ms = Some(v);
                pos = next;
            }
            b"instId" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                inst.check_symbol(v)?;
                pos = next;
            }
            _ => pos = skip_value(data, pos),
        }
    }
    require(have, SNAPSHOT_REQUIRED, KEYS)?;

    quote.with_quote_ext(quote_ext::SEQUENCE, seq_id);

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
    let mut prev_seq_id: u64 = 0;
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
            b"seqId" => {
                let (v, next) = parse_scalar_u64(data, pos);
                // One message, one sequence number: it is both ends of the
                // range this record covers.
                d.first_update_id = v;
                d.final_update_id = v;
                have |= seen::SEQUENCE;
                pos = next;
            }
            b"prevSeqId" => {
                let (v, next) = parse_scalar_u64(data, pos);
                prev_seq_id = v;
                have |= seen::PREV_SEQUENCE;
                pos = next;
            }
            b"ts" => {
                let (v, next) = parse_scalar_u64(data, pos);
                ts_ms = Some(v);
                pos = next;
            }
            b"instId" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                inst.check_symbol(v)?;
                pos = next;
            }
            _ => pos = skip_value(data, pos),
        }
    }
    require(have, DELTA_REQUIRED, KEYS)?;

    sides.finish(&mut d);
    d.with_prev_final_update_id(prev_seq_id);

    // Depth stays zero: the header's `depth` is a book's levels per side, and
    // a delta has neither a book nor sides of equal length. The counts are in
    // the payload.
    let mut h = inst.header(WireKind::SnapshotDelta, recv_ns);
    if let Some(ms) = ts_ms {
        h.set_venue_time(millis_to_nanos(ms));
    }

    *out = WireRecord::new_snapshot_delta(h, d);
    Ok(())
}
