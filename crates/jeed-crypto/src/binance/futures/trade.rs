//! `@aggTrade` — Binance USD-M aggregated prints.
//!
//! ```json
//! {"e":"aggTrade","E":1672515782136,"s":"BTCUSDT","a":164235345,
//!  "p":"0.001","q":"100","f":100,"l":105,"T":1672515782136,"m":true}
//! ```
//!
//! | key | |
//! |---|---|
//! | `T` | trade time (ms) → `venue_ns` |
//! | `p` `q` | price / **total** quantity of the aggregate |
//! | `m` | *was the buyer the maker* — `true` means the seller aggressed |
//! | `a` `f` `l` | aggregate id, first and last trade id — no wire slot |
//!
//! ## An aggregate trade is not a trade, and that is fine here
//!
//! One `@aggTrade` message is every fill at one price from one taker order.
//! USD-M offers nothing finer — there is no `@trade` stream — so this is the
//! print, and `q` is the size that moved.
//!
//! What it costs is the count: `f`..`l` is how many individual fills the
//! aggregate replaces, and neither that count nor the ids have a wire slot.
//! [`TradePayload::cumulative_qty`] is not it — that field is the session
//! total (KRX 누적체결수량), and filling it with an aggregate's size would
//! give the consumer a number that means something else on every other feed.
//!
//! [`TradePayload::cumulative_qty`]: jeed_wire::TradePayload::cumulative_qty

use crate::binance::{seen, seller_aggressed};
use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{next_key, parse_bool, parse_string_bytes, parse_u64, skip_value};
use crate::mask::require;
use crate::millis_to_nanos;
use jeed_wire::{BookPrice, BookQuantity, TradePayload, UnixNano, WireKind, WireRecord};

/// Every field the record needs.
const REQUIRED: u8 = seen::PRICE | seen::QTY | seen::MAKER;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[(seen::PRICE, "p"), (seen::QTY, "q"), (seen::MAKER, "m")];

/// Decodes an `@aggTrade` frame into `out`.
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
    let mut trade_ms: Option<u64> = None;
    let mut buyer_is_maker = false;
    let mut have = 0u8;

    let mut pos = 0;
    while pos < payload.len() {
        let (key, next) = next_key(payload, pos);
        pos = next;
        if key == 0 {
            break;
        }

        match key {
            b'p' => {
                let (v, next) = parse_string_bytes(payload, pos);
                price = inst.price(v, "p")?;
                have |= seen::PRICE;
                pos = next;
            }
            b'q' => {
                let (v, next) = parse_string_bytes(payload, pos);
                qty = inst.qty(v, "q")?;
                have |= seen::QTY;
                pos = next;
            }
            b'T' => {
                let (v, next) = parse_u64(payload, pos);
                trade_ms = Some(v);
                pos = next;
            }
            b'm' => {
                let (v, next) = parse_bool(payload, pos);
                buyer_is_maker = v;
                have |= seen::MAKER;
                pos = next;
            }
            b's' => {
                let (s, next) = parse_string_bytes(payload, pos);
                inst.check_symbol(s)?;
                pos = next;
            }
            _ => pos = skip_value(payload, pos),
        }
    }
    require(have, REQUIRED, KEYS)?;

    let mut trade = TradePayload::new(price, qty);
    trade.with_kind(seller_aggressed(buyer_is_maker));

    let mut h = inst.header(WireKind::Trade, recv_ns);
    if let Some(ms) = trade_ms {
        h.set_venue_time(millis_to_nanos(ms));
    }

    *out = WireRecord::new_trade(h, trade);
    Ok(())
}
