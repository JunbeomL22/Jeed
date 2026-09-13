//! `@trade` — Binance spot prints.
//!
//! ```json
//! {"e":"trade","E":1755088771745,"s":"ETHUSDT","t":2723467893,
//!  "p":"4712.06000000","q":"3.84410000","T":1755088771744,"m":true,"M":true}
//! ```
//!
//! | key | |
//! |---|---|
//! | `T` | trade time (ms) → `venue_ns`. **Not `E`**, which is when Binance's matching engine published the event, not when the trade happened |
//! | `p` `q` | price / quantity |
//! | `m` | *was the buyer the maker* — so `true` means the **seller** aggressed |
//! | `s` | symbol — checked, then dropped |
//! | `e` `t` `M` | event name, trade id, ignore flag — no wire slot |
//!
//! The trade id is not carried. It is a venue-scoped counter with no meaning
//! to the consumer, ordering authority is `producer_seq`, and the wire record
//! has no field for it (`documents/feed_handler.md` §8).

use crate::binance::{seen, seller_aggressed};
use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::{next_key, parse_bool, parse_string_bytes, parse_u64, skip_value};
use crate::mask::require;
use crate::millis_to_nanos;
use jeed_wire::{BookPrice, BookQuantity, TradePayload, UnixNano, WireKind, WireRecord};

/// Every field the record needs. `T` is not among them: a print with no time
/// is still a print, and the flag says so.
const REQUIRED: u8 = seen::PRICE | seen::QTY | seen::MAKER;

/// Which key each bit stands for, for the error message.
const KEYS: &[(u8, &str)] = &[(seen::PRICE, "p"), (seen::QTY, "q"), (seen::MAKER, "m")];

/// Decodes a `@trade` frame into `out`.
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
