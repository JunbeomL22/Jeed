//! Exchange JSON frames into wire records, no socket, no TLS.
//!
//! ```text
//! cargo run -p jeed-crypto --example decode
//! ```
//!
//! The frames below are what Binance spot and Upbit actually send. Each
//! decoder is pinned to one `Instrument` — venue, symbol, and the price and
//! quantity precision from the exchange's instrument reference — and fills one
//! `WireRecord`. The record is untouched on any error, so a caller that owns a
//! ring slot commits on `Ok` and abandons the slot otherwise.

use jeed_crypto::{CryptoError, Instrument, binance, upbit};
use jeed_wire::{UnixNano, Venue, WireKind, WireRecord, quote_ext, trade_kind};
use std::time::{SystemTime, UNIX_EPOCH};

/// Binance spot `@trade`. `T` is the trade time; `E` is when the event was
/// emitted, and is not what `venue_ns` carries.
const BINANCE_TRADE: &[u8] = br#"{"e":"trade","E":1755088771745,"s":"ETHUSDT","t":2723467893,"p":"4712.06000000","q":"3.84410000","T":1755088771744,"m":true,"M":true}"#;

/// Binance spot `@depth5` / `GET /api/v3/depth` — the same shape on both.
const BINANCE_BOOK: &[u8] = br#"{"lastUpdateId":7654321,"bids":[["4712.05000000","1.50000000"],["4712.04000000","0.25000000"],["4712.00000000","12.00000000"]],"asks":[["4712.06000000","2.00000000"],["4712.07000000","0.75000000"]]}"#;

/// The same frame with someone else's symbol — a wiring fault, refused.
const BINANCE_WRONG_SYMBOL: &[u8] = br#"{"e":"trade","E":1755088771745,"s":"BTCUSDT","t":1,"p":"117000.00000000","q":"0.01000000","T":1755088771744,"m":false,"M":true}"#;

/// Upbit `trade`. Numbers are unquoted, and above ten million the serialiser
/// switches to exponent notation — both are handled by the reader.
const UPBIT_TRADE: &[u8] = br#"{"type":"trade","code":"KRW-BTC","timestamp":1704067200123,"trade_date":"2024-01-01","trade_time":"00:00:00","trade_timestamp":1704067200000,"trade_price":1.5243E8,"trade_volume":0.00084280,"ask_bid":"BID","prev_closing_price":152000000.0,"change":"RISE","change_price":430000.0,"sequential_id":1704067200000000,"stream_type":"REALTIME"}"#;

fn now_ns() -> UnixNano {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64)
}

fn main() -> Result<(), CryptoError> {
    // Precision is configuration, from the exchange's instrument reference
    // (`exchangeInfo`): ETHUSDT ticks at 0.01 and steps at 0.0001. A frame with
    // more non-zero digits than that is refused rather than truncated.
    let eth = Instrument::new(Venue::BinanceSpot, b"ETHUSDT", 2, 4)?;
    let btc_krw = Instrument::new(Venue::Upbit, b"KRW-BTC", 0, 8)?;

    let mut rec = WireRecord::zeroed();

    println!("binance spot @trade");
    binance::spot::trade::decode(&eth, BINANCE_TRADE, now_ns(), &mut rec)?;
    show(&rec);

    println!("binance spot depth snapshot");
    binance::spot::snapshot::decode(&eth, BINANCE_BOOK, now_ns(), &mut rec)?;
    show(&rec);

    println!("upbit trade (unquoted, exponent-notation price)");
    upbit::trade::decode(&btc_krw, UPBIT_TRADE, now_ns(), &mut rec)?;
    show(&rec);

    println!("binance frame carrying another symbol — refused, record untouched");
    let before = rec;
    match binance::spot::trade::decode(&eth, BINANCE_WRONG_SYMBOL, now_ns(), &mut rec) {
        Ok(()) => println!("    unexpectedly accepted"),
        Err(e) => println!("    error: {e}"),
    }
    assert_eq!(rec.as_bytes()[..], before.as_bytes()[..], "a failed decode leaves the slot as it was");

    println!("a price with more precision than the instrument allows — refused, not truncated");
    let too_fine = br#"{"e":"trade","E":1,"s":"ETHUSDT","t":1,"p":"4712.06500000","q":"1.00000000","T":1,"m":true,"M":true}"#;
    match binance::spot::trade::decode(&eth, too_fine, now_ns(), &mut rec) {
        Ok(()) => println!("    unexpectedly accepted"),
        Err(e) => println!("    error: {e}"),
    }

    Ok(())
}

fn show(rec: &WireRecord) {
    let h = &rec.header;
    let ps = h.price_scale().unwrap();
    let qs = h.qty_scale().unwrap();
    let price = |p: i64| p as f64 * ps.multiplier();
    let qty = |q: u64| q as f64 * qs.multiplier();
    println!(
        "    -> {:?} {} {} price_scale={} qty_scale={} venue_time={:?}",
        rec.kind().unwrap(),
        h.venue().unwrap().as_str(),
        String::from_utf8_lossy(h.symbol_bytes()),
        ps.decimals(),
        qs.decimals(),
        h.venue_time()
    );
    match rec.kind() {
        Ok(WireKind::Trade) => {
            let t = rec.trade().unwrap();
            let side = match t.trade_kind {
                trade_kind::BUY => "buyer aggressed",
                trade_kind::SELL => "seller aggressed",
                _ => "aggressor unknown",
            };
            println!("       {} x {}  ({side})   raw price {} qty {}", price(t.price), qty(t.qty), t.price, t.qty);
        }
        Ok(WireKind::Quote) => {
            let q = rec.quote().unwrap();
            if q.quote_ext_kind == quote_ext::SEQUENCE {
                println!("       lastUpdateId {}  (what a delta chain resumes from)", q.quote_ext);
            }
            // `depth` is the deeper side; the shallower side's tail is zero.
            for (i, l) in q.bids(h.depth).iter().enumerate().filter(|(_, l)| l.qty != 0) {
                println!("       bid L{i}  {} x {}", price(l.price), qty(l.qty));
            }
            for (i, l) in q.asks(h.depth).iter().enumerate().filter(|(_, l)| l.qty != 0) {
                println!("       ask L{i}  {} x {}", price(l.price), qty(l.qty));
            }
        }
        _ => {}
    }
}
