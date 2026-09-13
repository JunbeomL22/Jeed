//! `spot.trades` — a Gate print.
//!
//! ```json
//! {"time":1606292218,"time_ms":1606292218231,"channel":"spot.trades",
//!  "event":"update",
//!  "result":{"id":309143071,"create_time":1606292218,
//!            "create_time_ms":"1606292218213.4578","side":"sell",
//!            "currency_pair":"GT_USDT","amount":"16.4700000000",
//!            "price":"0.4705000000"}}
//! ```
//!
//! | key | |
//! |---|---|
//! | `price` `amount` | price / quantity |
//! | `side` | `buy` or `sell` — the **taker's** side |
//! | `create_time_ms` | when the trade happened → `venue_ns` |
//! | `currency_pair` | symbol — checked, then dropped |
//! | `id` | venue trade id — no wire slot |
//! | `create_time` | the same instant, second-grained — not read |
//!
//! ## One print per frame, so no iterator
//!
//! `result` is a single object here, where OKX, Bybit and Bitget send an
//! array. The batched venues get an iterator because choosing one print out of
//! several is choosing which market moves to drop; there is nothing to choose
//! between when the venue sends one.
//!
//! ## The timestamp has a fraction, and it is kept
//!
//! `create_time_ms` is a quoted decimal — `"1606292218213.4578"` — where every
//! other venue in this crate sends an integer. Those digits are sub-millisecond
//! precision Gate actually measured, so they are read
//! ([`crate::time::decimal_millis_to_nanos`]) rather
//! than truncated at the point, which is what `fractal-engine` does.

use crate::error::CryptoError;
use crate::gate::seen;
use crate::instrument::Instrument;
use crate::json::{next_field, object_at, parse_scalar_bytes, skip_value};
use crate::mask::require;
use crate::time::decimal_millis_to_nanos;
use jeed_wire::{BookPrice, BookQuantity, TradePayload, UnixNano, WireKind, WireRecord, trade_kind};

/// Every field the record needs.
const REQUIRED: u8 = seen::PRICE | seen::QTY;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[(seen::PRICE, "price"), (seen::QTY, "amount")];

/// Decodes a `spot.trades` frame into `out`.
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

/// Fills a [`WireKind::Trade`] from the `result` object.
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
            b"amount" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                qty = inst.qty(v, "amount")?;
                have |= seen::QTY;
                pos = next;
            }
            b"side" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                kind = aggressor(v);
                pos = next;
            }
            b"create_time_ms" => {
                let (v, next) = parse_scalar_bytes(data, pos);
                trade_ns = decimal_millis_to_nanos(v);
                pos = next;
            }
            b"currency_pair" => {
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
