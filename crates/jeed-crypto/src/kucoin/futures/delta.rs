//! `/contractMarket/level2` — a KuCoin futures book diff, one level at a time.
//!
//! ```json
//! {"topic":"/contractMarket/level2:XBTUSDTM","type":"message","subject":"level2",
//!  "data":{"sequence":1709400450243,"change":"90631.2,sell,2",
//!          "timestamp":1731897467182}}
//! ```
//!
//! | key | |
//! |---|---|
//! | `change` | `price,side,size` — the whole diff, as a string |
//! | `sequence` | this message's sequence number, `+1` per message |
//! | `timestamp` | when the book changed, in **milliseconds** → `venue_ns` |
//! | `topic` | the only place the symbol appears |
//!
//! ## One level per message
//!
//! So a record carries one changed level and its side, and the overflow rule
//! that governs every other delta decoder can never fire here. `size` of `0`
//! deletes the price, as everywhere else.
//!
//! ## A side that cannot be read is refused
//!
//! `buy` puts the level on the bid side and `sell` on the ask. There is no
//! third possibility and no safe default: applying a change to the wrong side
//! leaves two prices wrong — one that should have moved and one that should
//! not have — and neither is corrected by any later message. A `change` this
//! build cannot take apart is therefore
//! [`crate::CryptoError::Unexpected`] and nothing is published.
//!
//! ## The sequence is a single number
//!
//! `sequence` advances by one per message, so `first_update_id` and
//! `final_update_id` are both it — the OKX shape rather than the spot channel's
//! range — and there is no named predecessor to carry.

use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{next_field, object_at, parse_scalar_bytes, parse_scalar_u64, skip_value};
use crate::kucoin::{check_topic, seen};
use crate::mask::require;
use crate::time::millis_to_nanos;
use jeed_wire::{SnapshotDeltaPayload, UnixNano, WireDeltaLevel, WireKind, WireRecord};

/// Every field the record needs.
const REQUIRED: u8 = seen::CHANGE | seen::SEQUENCE;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[(seen::CHANGE, "change"), (seen::SEQUENCE, "sequence")];

/// Decodes a `/contractMarket/level2` frame into `out`.
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
    let mut is_bid = false;
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
            b"change" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                let (level, bid) = change(inst, v)?;
                d.levels[0] = level;
                is_bid = bid;
                have |= seen::CHANGE;
                pos = next;
            }
            b"sequence" => {
                let (v, next) = parse_scalar_u64(data, pos);
                // One message, one sequence number: both ends of what it
                // covers.
                d.first_update_id = v;
                d.final_update_id = v;
                have |= seen::SEQUENCE;
                pos = next;
            }
            b"timestamp" => {
                let (v, next) = parse_scalar_u64(data, pos);
                book_ms = Some(v);
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

    if is_bid {
        d.bid_count = 1;
    } else {
        d.ask_count = 1;
    }

    // Depth stays zero: the header's `depth` is a book's levels per side, and
    // a delta has neither a book nor sides of equal length.
    let mut h = inst.header(WireKind::SnapshotDelta, recv_ns);
    if let Some(ms) = book_ms {
        h.set_venue_time(millis_to_nanos(ms));
    }

    *out = WireRecord::new_snapshot_delta(h, d);
    Ok(())
}

/// Reads `price,side,size` into a level and the side it belongs to.
///
/// `true` for a bid. An unparseable string — a missing comma, a side that is
/// neither `buy` nor `sell` — is
/// [`crate::CryptoError::Unexpected`]: there is nothing else in the message
/// to fall back on.
fn change(inst: &Instrument, value: &[u8]) -> Result<(WireDeltaLevel, bool), CryptoError> {
    let first = value.iter().position(|&b| b == b',').ok_or(unexpected())?;
    let rest = &value[first + 1..];
    let second = rest.iter().position(|&b| b == b',').ok_or(unexpected())?;

    let is_bid = match rest.first() {
        Some(b'b') => true,
        Some(b's') => false,
        _ => return Err(unexpected()),
    };

    let price = inst.price(&value[..first], "change")?;
    let qty = inst.qty(&rest[second + 1..], "change")?;

    Ok((WireDeltaLevel { price, qty }, is_bid))
}

/// The one error this file raises for a `change` it cannot read.
#[inline]
const fn unexpected() -> CryptoError {
    CryptoError::Unexpected { key: "change" }
}
