//! Tail a live ring — what a consumer process (an OMS) does.
//!
//! ```text
//! cargo run -p jeed-shm --example consumer                     # jeed.krx.hot
//! cargo run -p jeed-shm --example consumer -- jeed.crypto.hot  # any ring name
//! cargo run -p jeed-shm --example consumer -- jeed.krx.hot 20  # stop after 20 records
//! ```
//!
//! Start a producer first: `jeed-krx conf/krx.toml --no-pin`, `jeed-crypto …`,
//! or `cargo run -p jeed-shm --example roundtrip` in another terminal (its ring
//! is `jeed.example.roundtrip`; it exits quickly, so start this one first).
//!
//! The consumer depends on `jeed-wire` and `jeed-shm` only. It attaches at the
//! live edge, so it sees nothing published before it started.

use jeed_shm::{Recv, RingConsumer, SegmentName};
use jeed_wire::{WireKind, WireRecord, trade_kind};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let ring = args.next().unwrap_or_else(|| "jeed.krx.hot".to_owned());
    let limit: Option<u64> = args.next().map(|s| s.parse()).transpose()?;

    let name = SegmentName::local(&ring)?;
    let mut rx = match RingConsumer::attach(&name) {
        Ok(rx) => rx,
        Err(e) => {
            eprintln!("cannot attach to `{ring}`: {e}");
            eprintln!("is a producer running? try `cargo run -p jeed-shm --example roundtrip` in another terminal");
            std::process::exit(2);
        }
    };
    println!(
        "attached to `{ring}`: capacity {}  boot_id {:#x}  published {}  format v{}",
        rx.capacity(),
        rx.boot_id(),
        rx.published(),
        rx.header().format_version
    );

    let mut rec = WireRecord::zeroed();
    let mut seen = 0u64;
    loop {
        match rx.try_recv(&mut rec) {
            Recv::Record => {
                println!("{}", describe(&rec));
                seen += 1;
                if limit.is_some_and(|n| seen >= n) {
                    return Ok(());
                }
            }
            // Lost `n` records. A self-healing kind (Quote, Trade …) is fixed by
            // the next record; a SnapshotDelta chain now has a hole and must be
            // resynced from a whole book.
            Recv::Lagged(n) => println!("-- lagged: {n} records overwritten, now at the live edge"),
            Recv::Restarted { boot_id } => println!("-- producer restarted, boot_id {boot_id:#x}: discard everything built so far"),
            // Not a liveness signal: a quiet market and a dead producer look the
            // same here. Heartbeat records are what tell them apart.
            Recv::Empty => std::thread::yield_now(),
        }
    }
}

/// One line per record. Prices are integers on the wire; `price_scale` in the
/// header says where the decimal point goes.
fn describe(rec: &WireRecord) -> String {
    let h = &rec.header;
    let venue = h.venue().map_or("?", |v| v.as_str());
    let symbol = String::from_utf8_lossy(h.symbol_bytes());
    let price = |p: i64| p as f64 * h.price_scale().map_or(1.0, |s| s.multiplier());
    let qty = |q: u64| q as f64 * h.qty_scale().map_or(1.0, |s| s.multiplier());
    let stale = if h.is_stale() { " STALE" } else { "" };
    let head = format!("{:>10} {venue:<16} {symbol:<14}", h.producer_seq);

    match rec.kind() {
        Ok(WireKind::Quote) => {
            let q = rec.quote().unwrap();
            let bid = q.bids(h.depth).first().map_or("-".to_owned(), |l| format!("{} x {}", price(l.price), qty(l.qty)));
            let ask = q.asks(h.depth).first().map_or("-".to_owned(), |l| format!("{} x {}", price(l.price), qty(l.qty)));
            format!("{head} Quote  bid {bid}  ask {ask}  depth {}{stale}", h.depth)
        }
        Ok(WireKind::Trade) => {
            let t = rec.trade().unwrap();
            let side = match t.trade_kind {
                trade_kind::BUY => "buy ",
                trade_kind::SELL => "sell",
                _ => "?   ",
            };
            format!("{head} Trade  {side} {} x {}{stale}", price(t.price), qty(t.qty))
        }
        Ok(WireKind::TradeQuote) => {
            let tq = rec.trade_quote().unwrap();
            format!("{head} TradeQuote  {} x {}  + book depth {}{stale}", price(tq.trade.price), qty(tq.trade.qty), h.depth)
        }
        Ok(WireKind::SnapshotDelta) => {
            let d = rec.snapshot_delta().unwrap();
            format!(
                "{head} SnapshotDelta  {} bids {} asks  ids {}..{}",
                d.bid_count, d.ask_count, d.first_update_id, d.final_update_id
            )
        }
        Ok(WireKind::Heartbeat) => {
            let hb = rec.heartbeat().unwrap();
            format!("{head} Heartbeat  received {} forwarded {}", hb.received, hb.forwarded)
        }
        Ok(kind) => format!("{head} {kind:?}{stale}"),
        Err(e) => format!("{head} <invalid: {e}>"),
    }
}
