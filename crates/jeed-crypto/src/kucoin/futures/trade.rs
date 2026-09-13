//! `/contractMarket/execution` — a KuCoin futures print.
//!
//! ```json
//! {"topic":"/contractMarket/execution:XBTUSDTM","type":"message","subject":"match",
//!  "data":{"symbol":"XBTUSDTM","sequence":1743169228057,"side":"buy","size":5,
//!          "price":"72137.3","takerOrderId":"166754573174730752",
//!          "makerOrderId":"166754559966867456","tradeId":"1743169228057",
//!          "ts":1712570590399000000}}
//! ```
//!
//! | key | |
//! |---|---|
//! | `price` | quoted decimal |
//! | `size` | **unquoted integer** — contracts, not coins |
//! | `side` | `buy` or `sell` — the **taker's** side |
//! | `ts` | when the trade happened, in **nanoseconds**, unquoted → `venue_ns` |
//! | `symbol` | checked, then dropped |
//! | `sequence` `tradeId` `takerOrderId` `makerOrderId` | ids — no wire slot |
//!
//! ## Three spellings the spot channel does not use
//!
//! `size` is bare and whole, `ts` is bare and in nanoseconds, and the clock's
//! key is `ts` rather than `time`. Only the first of those changes anything
//! structural: a quantity scale of zero, set on the instrument, is what makes
//! `5` mean five contracts on the wire.

use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{next_field, object_at, parse_scalar_bytes, parse_scalar_u64, skip_value};
use crate::kucoin::{check_topic, seen};
use crate::mask::require;
use jeed_wire::{BookPrice, BookQuantity, TradePayload, UnixNano, WireKind, WireRecord, trade_kind};

/// Every field the record needs.
const REQUIRED: u8 = seen::PRICE | seen::QTY;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[(seen::PRICE, "price"), (seen::QTY, "size")];

/// Decodes a `/contractMarket/execution` frame into `out`.
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

/// Fills a [`WireKind::Trade`] from the `data` object.
fn body(
    inst: &Instrument,
    data: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut price: BookPrice = 0;
    let mut qty: BookQuantity = 0;
    let mut kind = trade_kind::UNKNOWN;
    let mut trade_ns: Option<UnixNano> = None;
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
                // Already nanoseconds, as on the spot channel — where the key
                // is `time` instead.
                let (v, next) = parse_scalar_u64(data, pos);
                trade_ns = Some(v);
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

    let mut trade = TradePayload::new(price, qty);
    trade.with_kind(kind);

    let mut h = inst.header(WireKind::Trade, recv_ns);
    if let Some(ns) = trade_ns {
        h.set_venue_time(ns);
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
