//! `orderbook` — an Upbit book, whole, every time.
//!
//! ```json
//! {"type":"orderbook","code":"KRW-BTC","timestamp":1704067200000,
//!  "total_ask_size":1.234,"total_bid_size":2.345,
//!  "orderbook_units":[
//!    {"ask_price":152430000.0,"bid_price":152400000.0,
//!     "ask_size":0.17,"bid_size":0.23}],
//!  "stream_type":"REALTIME","level":0}
//! ```
//!
//! | DEFAULT | SIMPLE | |
//! |---|---|---|
//! | `code` | `cd` | market code — checked against the instrument, then dropped |
//! | `timestamp` | `tms` | message time (ms) → `venue_ns` |
//! | `orderbook_units` | `obu` | one entry per rank, both sides in it |
//! | `ask_price` `bid_price` | `ap` `bp` | prices at that rank |
//! | `ask_size` `bid_size` | `as` `bs` | sizes at that rank |
//! | `total_ask_size` `total_bid_size` | `tas` `tbs` | side totals — no wire slot |
//! | `stream_type` | `st` | `SNAPSHOT` on subscribe, `REALTIME` after |
//! | `level` | `lv` | price grouping unit, `0` for ungrouped |
//!
//! ## Paired units, two independent depths
//!
//! Unit *i* holds the bid and the ask at rank *i*, so the sides are read
//! together and counted apart: a thin market can run out of asks four ranks
//! before it runs out of bids, and both sides then arrive padded with zeros to
//! the subscribed length. Counting levels that carry quantity rather than
//! levels that arrived is the same rule
//! [`BookShape`] applies everywhere else.
//!
//! [`BookShape`]: crate::BookShape
//!
//! ## `stream_type` is not carried
//!
//! Upbit marks the first message after a subscribe `SNAPSHOT` and the rest
//! `REALTIME`. The distinction is about the subscription's age, not the book's
//! content — both are the whole book — and a consumer that branched on it
//! would be reading a fact about the handler's connection. It is skipped.

use crate::book::BookShape;
use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{
    next_field, object_at, parse_scalar_bytes, parse_scalar_u64, skip_value, skip_ws,
};
use crate::mask::require;
use crate::millis_to_nanos;
use crate::upbit::seen;
use jeed_wire::{
    QuotePayload, UnixNano, WIRE_MAX_DEPTH, WireKind, WireLevel, WireRecord,
};

/// Every field the record needs. Not the timestamp: a book with no time is
/// still a book, and the flag says so.
const REQUIRED: u8 = seen::UNITS;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[(seen::UNITS, "orderbook_units")];

/// Decodes an `orderbook` frame into `out`.
///
/// `out` is left untouched on error.
pub fn decode(
    inst: &Instrument,
    payload: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut quote = QuotePayload::default();
    let mut event_ms: Option<u64> = None;
    let mut bid_depth = 0u8;
    let mut ask_depth = 0u8;
    let mut have = 0u8;

    let mut pos = 0;
    while pos < payload.len() {
        let (key, next) = next_field(payload, pos);
        pos = next;
        if key.is_empty() {
            break;
        }

        match key {
            b"timestamp" | b"tms" => {
                let (v, next) = parse_scalar_u64(payload, pos);
                event_ms = Some(v);
                pos = next;
            }
            b"orderbook_units" | b"obu" => {
                let (b, a, next) = units(inst, payload, pos, &mut quote)?;
                bid_depth = b;
                ask_depth = a;
                have |= seen::UNITS;
                pos = next;
            }
            b"code" | b"cd" => {
                let (s, next) = parse_scalar_bytes(payload, pos);
                inst.check_symbol(s)?;
                pos = next;
            }
            _ => pos = skip_value(payload, pos),
        }
    }
    require(have, REQUIRED, KEYS)?;

    let shape = BookShape::new(bid_depth, ask_depth);
    let mut h = inst.header(WireKind::Quote, recv_ns);
    h.set_depth(shape.depth).set_flags(shape.flags);
    if let Some(ms) = event_ms {
        h.set_venue_time(millis_to_nanos(ms));
    }

    *out = WireRecord::new_quote(h, quote);
    Ok(())
}

/// Reads `[{ask_price,bid_price,ask_size,bid_size}, …]` into both sides.
///
/// Returns `(bid depth, ask depth, position past the array)`. Ranks past
/// [`WIRE_MAX_DEPTH`] are skipped without being parsed, as in every other
/// snapshot decoder: a subscription can ask for thirty and the wire carries
/// ten.
fn units(
    inst: &Instrument,
    data: &[u8],
    pos: usize,
    quote: &mut QuotePayload,
) -> Result<(u8, u8, usize), CryptoError> {
    let open = skip_ws(data, pos);
    if open >= data.len() || data[open] != b'[' {
        return Err(CryptoError::Missing { key: "orderbook_units" });
    }
    let mut pos = open + 1;

    let mut rank = 0usize;
    let mut bid_depth = 0usize;
    let mut ask_depth = 0usize;

    loop {
        pos = skip_ws(data, pos);
        if pos >= data.len() {
            break;
        }
        match data[pos] {
            b']' => {
                pos += 1;
                break;
            }
            b',' => pos += 1,
            b'{' => {
                let Some((inner, past)) = object_at(data, pos) else {
                    return Err(CryptoError::Missing { key: "orderbook_unit" });
                };
                pos = past;

                if rank < WIRE_MAX_DEPTH {
                    let (bid, ask) = unit(inst, inner)?;
                    quote.set_bid(rank, bid).set_ask(rank, ask);
                    rank += 1;
                    if bid.qty > 0 {
                        bid_depth = rank;
                    }
                    if ask.qty > 0 {
                        ask_depth = rank;
                    }
                }
            }
            _ => pos += 1,
        }
    }

    Ok((bid_depth as u8, ask_depth as u8, pos))
}

/// Reads one unit object's four numbers.
///
/// A missing price or size reads as zero rather than failing. Upbit sends all
/// four on every unit, and a level whose quantity is zero is already how both
/// this decoder and the wire spell *nothing here* — so the mask that guards
/// the top-level keys would be measuring the same thing twice.
fn unit(inst: &Instrument, data: &[u8]) -> Result<(WireLevel, WireLevel), CryptoError> {
    let mut bid = WireLevel::default();
    let mut ask = WireLevel::default();

    let mut pos = 0;
    while pos < data.len() {
        let (key, next) = next_field(data, pos);
        pos = next;
        if key.is_empty() {
            break;
        }

        match key {
            b"ask_price" | b"ap" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                ask.price = inst.price(v, "ask_price")?;
                pos = next;
            }
            b"bid_price" | b"bp" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                bid.price = inst.price(v, "bid_price")?;
                pos = next;
            }
            b"ask_size" | b"as" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                ask.qty = inst.qty(v, "ask_size")?;
                pos = next;
            }
            b"bid_size" | b"bs" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                bid.qty = inst.qty(v, "bid_size")?;
                pos = next;
            }
            _ => pos = skip_value(data, pos),
        }
    }

    Ok((bid, ask))
}
