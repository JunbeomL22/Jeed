//! `trade` — an Upbit print.
//!
//! ```json
//! {"type":"trade","code":"KRW-BTC","timestamp":1704067200123,
//!  "trade_date":"2024-01-01","trade_time":"00:00:00",
//!  "trade_timestamp":1704067200000,
//!  "trade_price":52430000.0,"trade_volume":0.00084280,"ask_bid":"BID",
//!  "prev_closing_price":52000000.0,"change":"RISE","change_price":430000.0,
//!  "sequential_id":1704067200000000,"stream_type":"REALTIME"}
//! ```
//!
//! | DEFAULT | SIMPLE | |
//! |---|---|---|
//! | `trade_price` | `tp` | price |
//! | `trade_volume` | `tv` | quantity |
//! | `ask_bid` | `ab` | which side took liquidity |
//! | `trade_timestamp` | `ttms` | when the trade happened (ms) → `venue_ns` |
//! | `timestamp` | `tms` | when the message was pushed — **not** used |
//! | `code` | `cd` | market code — checked, then dropped |
//! | `sequential_id` | `sid` | venue trade id — no wire slot |
//! | `change` `change_price` `prev_closing_price` | `c` `cp` `pcp` | daily statistics — no wire slot |
//!
//! ## `ask_bid` already names the aggressor
//!
//! Where Binance sends `m` — *was the buyer the maker* — and leaves the
//! aggressor to be worked out from it, Upbit names the side that took
//! liquidity outright: `ASK` is an aggressive seller, `BID` an aggressive
//! buyer. Both end up in the same [`trade_kind`] byte,
//! which is the point of carrying a conclusion rather than the venue's
//! spelling of it.
//!
//! A third value is not a decode failure. The aggressor is one field of a
//! print and [`trade_kind::UNKNOWN`] exists to
//! say the flag was there and said nothing — dropping the price and size over
//! it would lose more than it protects.
//!
//! ## `trade_timestamp`, not `timestamp`
//!
//! One says when the trade happened, the other when Upbit got round to
//! pushing it. The same choice `binance::spot::trade` makes between `T` and
//! `E`, and for the same reason: `venue_ns` is meant to be comparable with
//! another venue's print of the same market move.
//!
//! ## The trade id is not carried
//!
//! `sequential_id` is unique and monotonic per market, which makes it useful
//! for de-duplicating a replay — and it is still a venue-scoped counter with
//! no meaning to the consumer, ordering authority is `producer_seq`, and the
//! wire record has no field for it (`documents/feed_handler.md` §8). Binance's
//! `t` is dropped for the same reason.

use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{next_field, parse_scalar_bytes, parse_scalar_u64, skip_value};
use crate::mask::require;
use crate::millis_to_nanos;
use crate::upbit::seen;
use jeed_wire::{
    BookPrice, BookQuantity, TradePayload, UnixNano, WireKind, WireRecord, trade_kind,
};

/// Every field the record needs.
const REQUIRED: u8 = seen::PRICE | seen::QTY | seen::SIDE;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] =
    &[(seen::PRICE, "trade_price"), (seen::QTY, "trade_volume"), (seen::SIDE, "ask_bid")];

/// Decodes a `trade` frame into `out`.
///
/// `out` is left untouched on error.
pub fn decode(
    inst: &Instrument,
    payload: &[u8],
    recv_ns: UnixNano,
    out: &mut WireRecord,
) -> Result<(), CryptoError> {
    let mut price: BookPrice = 0;
    let mut qty: BookQuantity = 0;
    let mut kind = trade_kind::UNKNOWN;
    let mut trade_ms: Option<u64> = None;
    let mut have = 0u8;

    let mut pos = 0;
    while pos < payload.len() {
        let (key, next) = next_field(payload, pos);
        pos = next;
        if key.is_empty() {
            break;
        }

        match key {
            b"trade_price" | b"tp" => {
                let (v, next) = parse_scalar_bytes(payload, pos);
                price = inst.price(v, "trade_price")?;
                have |= seen::PRICE;
                pos = next;
            }
            b"trade_volume" | b"tv" => {
                let (v, next) = parse_scalar_bytes(payload, pos);
                qty = inst.qty(v, "trade_volume")?;
                have |= seen::QTY;
                pos = next;
            }
            b"ask_bid" | b"ab" => {
                let (v, next) = parse_scalar_bytes(payload, pos);
                kind = aggressor(v);
                have |= seen::SIDE;
                pos = next;
            }
            b"trade_timestamp" | b"ttms" => {
                let (v, next) = parse_scalar_u64(payload, pos);
                trade_ms = Some(v);
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

    let mut trade = TradePayload::new(price, qty);
    trade.with_kind(kind);

    let mut h = inst.header(WireKind::Trade, recv_ns);
    if let Some(ms) = trade_ms {
        h.set_venue_time(millis_to_nanos(ms));
    }

    *out = WireRecord::new_trade(h, trade);
    Ok(())
}

/// `ASK` / `BID` as a [`trade_kind`] byte.
#[inline]
fn aggressor(side: &[u8]) -> u8 {
    match side {
        b"ASK" => trade_kind::SELL,
        b"BID" => trade_kind::BUY,
        _ => trade_kind::UNKNOWN,
    }
}
