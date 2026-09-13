//! `publicTrade.{symbol}` — Bybit prints, several to a frame.
//!
//! ```json
//! {"topic":"publicTrade.BTCUSDT","type":"snapshot","ts":1672304486868,
//!  "data":[{"T":1672304486865,"s":"BTCUSDT","S":"Buy","v":"0.001",
//!           "p":"16578.50","L":"PlusTick",
//!           "i":"20f43950-d8dd-5b31-9112-a178eb6023af","BT":false}]}
//! ```
//!
//! | key | |
//! |---|---|
//! | `p` `v` | price / quantity |
//! | `S` | `Buy` or `Sell` — the **taker's** side |
//! | `T` | when the trade happened (ms) → `venue_ns` |
//! | `s` | symbol — checked, then dropped |
//! | `L` `i` `BT` | tick direction, trade id, block-trade flag — no wire slot |
//!
//! ## `type` is always `snapshot`, and means nothing here
//!
//! Bybit types every `publicTrade` frame `snapshot`. On the orderbook channel
//! that word routes the frame; on this one it is furniture, and reading it
//! would only invite the mistake of thinking a print stream had snapshot and
//! delta forms. It is skipped.
//!
//! ## A frame is a batch, so the API is a batch
//!
//! One frame holds every print in the same push, so this hands out an iterator
//! that fills a record at a time rather than choosing one print and dropping
//! the rest — the same shape [`okx::trade`](crate::okx::trade) uses, and the
//! same shape the ring wants: claim a slot, fill it, publish, repeat.
//!
//! ```
//! use jeed_crypto::{Instrument, bybit};
//! use jeed_wire::{Venue, WireRecord};
//!
//! let inst = Instrument::new(Venue::BybitLinear, b"BTCUSDT", 2, 3)?;
//! let frame = br#"{"topic":"publicTrade.BTCUSDT","type":"snapshot","ts":1672304486868,
//!     "data":[{"T":1672304486865,"s":"BTCUSDT","S":"Buy","v":"0.001","p":"16578.50"},
//!             {"T":1672304486866,"s":"BTCUSDT","S":"Sell","v":"0.002","p":"16578.00"}]}"#;
//!
//! let mut rec = WireRecord::zeroed();
//! let mut prints = bybit::trade::trades(frame)?;
//! let mut n = 0;
//! while let Some(result) = prints.decode_next(&inst, 1_700_000_000_000_000_000, &mut rec) {
//!     result?;
//!     n += 1;
//! }
//! assert_eq!(n, 2);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! ## `S` already names the aggressor
//!
//! Bybit reports the taker's side outright — no `m`-style *was the buyer the
//! maker* to invert. Matched on the first byte, which is why the case matters:
//! `S` is the side and `s` is the symbol, and they are different keys.

use crate::bybit::seen;
use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{Objects, next_field, objects_at, parse_scalar_bytes, parse_scalar_u64, skip_value};
use crate::mask::require;
use crate::millis_to_nanos;
use jeed_wire::{
    BookPrice, BookQuantity, TradePayload, UnixNano, WireKind, WireRecord, trade_kind,
};

/// Every field the record needs.
const REQUIRED: u8 = seen::PRICE | seen::QTY;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[(seen::PRICE, "p"), (seen::QTY, "v")];

/// The prints in one `publicTrade` frame.
#[derive(Debug, Clone)]
pub struct Trades<'a> {
    /// The `data` array's objects.
    objects: Objects<'a>,
}

/// Opens a `publicTrade` frame's `data` array.
///
/// Fails only when there is no `data` array at all; an empty one yields no
/// prints, which is not an error.
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
    /// `out` is left untouched on error, and iteration can continue past one.
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
    let mut trade_ms: Option<u64> = None;
    let mut have = 0u8;

    let mut pos = 0;
    while pos < data.len() {
        let (key, next) = next_field(data, pos);
        pos = next;
        if key.is_empty() {
            break;
        }

        match key {
            b"p" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                price = inst.price(v, "p")?;
                have |= seen::PRICE;
                pos = next;
            }
            b"v" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                qty = inst.qty(v, "v")?;
                have |= seen::QTY;
                pos = next;
            }
            b"S" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                kind = aggressor(v);
                pos = next;
            }
            b"T" => {
                let (v, next) = parse_scalar_u64(data, pos);
                trade_ms = Some(v);
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

    let mut trade = TradePayload::new(price, qty);
    trade.with_kind(kind);

    let mut h = inst.header(WireKind::Trade, recv_ns);
    if let Some(ms) = trade_ms {
        h.set_venue_time(millis_to_nanos(ms));
    }

    *out = WireRecord::new_trade(h, trade);
    Ok(())
}

/// `Buy` / `Sell` as a [`trade_kind`] byte.
#[inline]
fn aggressor(side: &[u8]) -> u8 {
    match side.first() {
        Some(b'B') => trade_kind::BUY,
        Some(b'S') => trade_kind::SELL,
        _ => trade_kind::UNKNOWN,
    }
}
