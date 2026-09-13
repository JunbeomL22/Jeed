//! `trades` — OKX prints, several to a frame.
//!
//! ```json
//! {"arg":{"channel":"trades","instId":"BTC-USDT"},
//!  "data":[{"instId":"BTC-USDT","tradeId":"130639474","px":"42219.9",
//!           "sz":"0.12","side":"buy","ts":"1630048459987","count":"3"}]}
//! ```
//!
//! | key | |
//! |---|---|
//! | `px` `sz` | price / quantity |
//! | `side` | `buy` or `sell` — the **taker's** side |
//! | `ts` | when the trade happened (ms, quoted) → `venue_ns` |
//! | `instId` | checked against the instrument, then dropped |
//! | `tradeId` `count` | venue trade id, orders filled — no wire slot |
//!
//! ## A frame is a batch, so the API is a batch
//!
//! The `trades` channel aggregates: one frame can hold every print a single
//! taker order caused, and `trades-all` holds more still. There is one print
//! per [`WireRecord`], so a `decode` that took one frame and one record would
//! have to choose which print to keep — `fractal-engine` kept the last one and
//! dropped the rest. Every print moved the tape; none of them is optional.
//!
//! So this hands out an iterator that fills a record at a time, which is also
//! the shape the ring wants: claim a slot, fill it, publish, repeat.
//!
//! ```
//! use jeed_crypto::{Instrument, okx};
//! use jeed_wire::{Venue, WireRecord};
//!
//! let inst = Instrument::new(Venue::Okx, b"BTC-USDT", 1, 2)?;
//! let frame = br#"{"arg":{"channel":"trades","instId":"BTC-USDT"},"data":[
//!     {"instId":"BTC-USDT","px":"42219.9","sz":"0.12","side":"buy","ts":"1630048459987"},
//!     {"instId":"BTC-USDT","px":"42220.0","sz":"0.03","side":"sell","ts":"1630048459988"}]}"#;
//!
//! let mut rec = WireRecord::zeroed();
//! let mut prints = okx::trade::trades(frame)?;
//! let mut n = 0;
//! while let Some(result) = prints.decode_next(&inst, 1_700_000_000_000_000_000, &mut rec) {
//!     result?;
//!     n += 1;
//! }
//! assert_eq!(n, 2);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! ## `side` already names the aggressor
//!
//! OKX reports the taker's side outright, so there is no `m`-style *was the
//! buyer the maker* to invert the way Binance needs. Matched on the first byte
//! because `buy` and `sell` are the only two values, and an unrecognised one
//! becomes [`trade_kind::UNKNOWN`] rather than a dropped print — a price and a
//! size are worth more than the flag that went missing.

use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{Objects, next_field, objects_at, parse_scalar_bytes, parse_scalar_u64, skip_value};
use crate::mask::require;
use crate::millis_to_nanos;
use crate::okx::seen;
use jeed_wire::{
    BookPrice, BookQuantity, TradePayload, UnixNano, WireKind, WireRecord, trade_kind,
};

/// Every field the record needs.
const REQUIRED: u8 = seen::PRICE | seen::QTY;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[(seen::PRICE, "px"), (seen::QTY, "sz")];

/// The prints in one `trades` frame.
///
/// Built by [`trades`]; borrows the frame, allocates nothing, and holds no
/// instrument — the instrument is supplied per record so one frame's prints
/// can be published as fast as slots come free.
#[derive(Debug, Clone)]
pub struct Trades<'a> {
    /// The `data` array's objects.
    objects: Objects<'a>,
}

/// Opens a `trades` frame's `data` array.
///
/// Fails only when there is no `data` array at all. An array that is present
/// and empty yields no prints, which is not an error — OKX sends one on
/// subscribe.
pub fn trades(payload: &[u8]) -> Result<Trades<'_>, CryptoError> {
    let mut pos = 0;
    while pos < payload.len() {
        let (key, next) = next_field(payload, pos);
        pos = next;
        if key.is_empty() {
            break;
        }
        if key == b"data" {
            let objects = objects_at(payload, pos).ok_or(CryptoError::Missing { key: "data" })?;
            return Ok(Trades { objects });
        }
        pos = skip_value(payload, pos);
    }
    Err(CryptoError::Missing { key: "data" })
}

impl Trades<'_> {
    /// Decodes the next print into `out`, or `None` when the frame is spent.
    ///
    /// `out` is left untouched on error, and the iteration can continue past
    /// one: a frame's later prints are not made wrong by an earlier one
    /// failing to parse.
    pub fn decode_next(
        &mut self,
        inst: &Instrument,
        recv_ns: UnixNano,
        out: &mut WireRecord,
    ) -> Option<Result<(), CryptoError>> {
        let entry = self.objects.next()?;
        Some(decode_entry(inst, entry, recv_ns, out))
    }
}

/// Decodes one `data` element.
fn decode_entry(
    inst: &Instrument,
    data: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut price: BookPrice = 0;
    let mut qty: BookQuantity = 0;
    let mut kind = trade_kind::UNKNOWN;
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
            b"px" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                price = inst.price(v, "px")?;
                have |= seen::PRICE;
                pos = next;
            }
            b"sz" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                qty = inst.qty(v, "sz")?;
                have |= seen::QTY;
                pos = next;
            }
            b"side" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                kind = aggressor(v);
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
    require(have, REQUIRED, KEYS)?;

    let mut trade = TradePayload::new(price, qty);
    trade.with_kind(kind);

    let mut h = inst.header(WireKind::Trade, recv_ns);
    if let Some(ms) = ts_ms {
        h.set_venue_time(millis_to_nanos(ms));
    }

    *out = WireRecord::new_trade(h, trade);
    Ok(())
}

/// `buy` / `sell` as a [`trade_kind`] byte.
#[inline]
fn aggressor(side: &[u8]) -> u8 {
    match side.first() {
        Some(b'b') => trade_kind::BUY,
        Some(b's') => trade_kind::SELL,
        _ => trade_kind::UNKNOWN,
    }
}
