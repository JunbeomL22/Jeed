//! `trade` — Bitget prints, several to a frame.
//!
//! ```json
//! {"action":"snapshot",
//!  "arg":{"instType":"USDT-FUTURES","channel":"trade","instId":"BTCUSDT"},
//!  "data":[{"ts":"1695716760565","price":"27000.5","size":"0.001",
//!           "side":"buy","tradeId":"1111111111"}]}
//! ```
//!
//! | key | |
//! |---|---|
//! | `price` `size` | price / quantity |
//! | `side` | `buy` or `sell` — the **taker's** side |
//! | `ts` | when the trade happened (ms, quoted) → `venue_ns` |
//! | `tradeId` | venue trade id — no wire slot |
//! | `action` | `snapshot` on subscribe, `update` after — furniture here |
//!
//! ## The symbol is in the envelope, so the envelope is checked
//!
//! A `data` entry carries no symbol at all — only `arg.instId` does. That is
//! why [`trades`] takes the [`Instrument`] where
//! [`okx::trade::trades`](crate::okx::trade::trades) does not: OKX repeats
//! `instId` on every print and can check each one as it is decoded, while here
//! the only chance to catch a mis-wired subscription is the frame itself.
//! Checking once per frame instead of once per print is also the cheaper end
//! of that trade.
//!
//! ## `action` decides nothing on this channel
//!
//! Bitget types the first `trade` frame after a subscribe `snapshot` and the
//! rest `update`, which on the book channel routes the frame and here means
//! only *this is the backfill*. Both are prints that happened; neither
//! replaces the other. It is skipped, as Bybit's always-`snapshot` `type` is.
//!
//! ## A frame is a batch, so the API is a batch
//!
//! One frame holds every print in the same push, so this hands out an iterator
//! that fills a record at a time rather than choosing one print and dropping
//! the rest — the shape [`okx::trade`](crate::okx::trade) uses, and the shape
//! the ring wants: claim a slot, fill it, publish, repeat.
//!
//! ```
//! use jeed_crypto::{Instrument, bitget};
//! use jeed_wire::{Venue, WireRecord};
//!
//! let inst = Instrument::new(Venue::BitgetLinear, b"BTCUSDT", 1, 3)?;
//! let frame = br#"{"action":"update",
//!     "arg":{"instType":"USDT-FUTURES","channel":"trade","instId":"BTCUSDT"},
//!     "data":[{"ts":"1695716760565","price":"27000.5","size":"0.001","side":"buy"},
//!             {"ts":"1695716760566","price":"27000.0","size":"0.002","side":"sell"}]}"#;
//!
//! let mut rec = WireRecord::zeroed();
//! let mut prints = bitget::trade::trades(&inst, frame)?;
//! let mut n = 0;
//! while let Some(result) = prints.decode_next(&inst, 1_700_000_000_000_000_000, &mut rec) {
//!     result?;
//!     n += 1;
//! }
//! assert_eq!(n, 2);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! [`Instrument`]: crate::Instrument

use crate::bitget::book::check_inst_id;
use crate::bitget::seen;
use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{
    Objects, next_field, object_at, objects_at, parse_scalar_bytes, parse_scalar_u64, skip_value,
};
use crate::mask::require;
use crate::time::millis_to_nanos;
use jeed_wire::{BookPrice, BookQuantity, TradePayload, UnixNano, WireKind, WireRecord, trade_kind};

/// Every field the record needs.
const REQUIRED: u8 = seen::PRICE | seen::QTY;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[(seen::PRICE, "price"), (seen::QTY, "size")];

/// The prints in one `trade` frame.
#[derive(Debug, Clone)]
pub struct Trades<'a> {
    /// The `data` array's objects.
    objects: Objects<'a>,
}

/// Opens a `trade` frame's `data` array, checking `arg.instId` on the way.
///
/// Fails when there is no `data` array, or when the frame is for another
/// instrument. An array that is present and empty yields no prints, which is
/// not an error.
pub fn trades<'a>(inst: &Instrument, payload: &'a [u8]) -> Result<Trades<'a>, CryptoError> {
    let mut objects: Option<Objects<'a>> = None;

    let mut pos = 0;
    while pos < payload.len() {
        let (key, next) = next_field(payload, pos);
        pos = next;
        if key.is_empty() {
            break;
        }

        match key {
            b"data" => {
                objects = objects_at(payload, pos);
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

    objects.map(|objects| Trades { objects }).ok_or(CryptoError::Missing { key: "data" })
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
            b"price" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                price = inst.price(v, "price")?;
                have |= seen::PRICE;
                pos = next;
            }
            b"size" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                qty = inst.qty(v, "size")?;
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
                trade_ms = Some(v);
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

/// `buy` / `sell` as a [`trade_kind`] byte.
#[inline]
fn aggressor(side: &[u8]) -> u8 {
    match side.first() {
        Some(b'b') => trade_kind::BUY,
        Some(b's') => trade_kind::SELL,
        _ => trade_kind::UNKNOWN,
    }
}
