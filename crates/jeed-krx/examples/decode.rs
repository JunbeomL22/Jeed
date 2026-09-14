//! Two KRX datagrams through the pipeline, no socket, no shared memory.
//!
//! ```text
//! cargo run -p jeed-krx --example decode
//! ```
//!
//! The bytes below are a KOSPI200 futures book (`B601F`, 324 B) and a print
//! (`A301F`, 173 B), built from the 정보분배 interface definitions exactly as
//! the decoder tests build them. `Pipeline::ingest` is the same call the
//! receive loop and `jeed-pcap` make, so what happens here happens on the wire.
//!
//! The sink is a stand-in for the ring: the decoders write into a
//! `jeed_wire::RecordSink` and never see shared memory.

use jeed_krx::TrCode;
use jeed_krx::recv::{IsinFilter, Outcome, Pipeline, TrCodeFilter};
use jeed_wire::{RecordSink, UnixNano, WireKind, WireRecord, trade_kind};
use std::time::{SystemTime, UNIX_EPOCH};

/// `B601F` 우선호가 — KOSPI200 선물 `KR4101V90009`, five levels, best 937.05 / 936.95.
const BOOK: &[u8] = b"B601F00000001G140KR4101V90009000001090100123456000937.05000936.950000000100000000080000300002000937.10000936.900000000250000000310000700009000937.15000936.850000000400000000440001100012000937.20000936.800000000550000000600001500016000937.25000936.7500000007000000007700019000200000000000000000000000000000000000.00000000000\xff";

/// `A301F` 체결 — the same contract printing 3 lots at 937.00, dynamic band 946.35 / 927.65.
const TRADE: &[u8] = b"A301F00000001G140KR4101V90009000001090100123456000937.00000000003000000.00000000.00000935.10000938.40000933.75000936.95000000166478000000000155908543.0002000946.35000927.65\xff";

fn now_ns() -> UnixNano {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64)
}

/// Prints every committed record. Like a ring slot, the buffer is reused and
/// never cleared: a decoder that fails half-way leaves nothing published.
struct Print {
    slot: WireRecord,
}

impl RecordSink for Print {
    fn publish<E>(&mut self, fill: impl FnOnce(&mut WireRecord) -> Result<(), E>) -> Result<(), E> {
        fill(&mut self.slot)?;
        show(&self.slot);
        Ok(())
    }

    fn note_drops(&mut self, n: u64) {
        println!("    sink: {n} dropped before the ring");
    }
}

fn show(rec: &WireRecord) {
    let h = &rec.header;
    let scale = h.price_scale().map_or(1.0, |s| s.multiplier());
    let symbol = String::from_utf8_lossy(h.symbol_bytes());
    println!(
        "    -> {:?} {} venue={} price_scale={} depth={} venue_time={:?}",
        rec.kind().unwrap(),
        symbol,
        h.venue().map_or("?", |v| v.as_str()),
        h.price_scale().map_or(0, |s| s.decimals()),
        h.depth,
        h.venue_time()
    );
    match rec.kind() {
        Ok(WireKind::Quote) => {
            let q = rec.quote().unwrap();
            for (i, (bid, ask)) in q.bids(h.depth).iter().zip(q.asks(h.depth)).enumerate() {
                println!(
                    "       L{i}  bid {:>8.2} x {:<4} ({} orders)   ask {:>8.2} x {:<4} ({} orders)",
                    bid.price as f64 * scale,
                    bid.qty,
                    bid.order_count,
                    ask.price as f64 * scale,
                    ask.qty,
                    ask.order_count
                );
            }
        }
        Ok(WireKind::Trade) => {
            let t = rec.trade().unwrap();
            let side = match t.trade_kind {
                trade_kind::BUY => "buyer aggressed",
                trade_kind::SELL => "seller aggressed",
                _ => "aggressor unknown",
            };
            println!("       {:.2} x {} ({side})  cumulative {:?}", t.price as f64 * scale, t.qty, t.cumulative_qty());
            if let Some((hi, lo)) = t.dyn_limits() {
                println!("       dynamic band {:.2} .. {:.2}", lo as f64 * scale, hi as f64 * scale);
            }
        }
        _ => {}
    }
}

fn code(s: &str) -> TrCode {
    TrCode::from_message(s.as_bytes()).expect("five bytes")
}

fn main() {
    // What to keep. A KRX port carries every 데이터구분 of its product group
    // mixed together; the filter is what turns a socket into a feed.
    let trcodes = TrCodeFilter::new([code("B601F"), code("A301F")]);
    let mut pipe = Pipeline::new(trcodes, IsinFilter::all(), 0, Print { slot: WireRecord::zeroed() });

    println!("B601F, {} bytes", BOOK.len());
    report(pipe.ingest(BOOK, now_ns()));

    println!("A301F, {} bytes", TRADE.len());
    report(pipe.ingest(TRADE, now_ns()));

    println!("G701F is not in the filter — the rest of the port's traffic, not a fault");
    let mut other = BOOK.to_vec();
    other[..5].copy_from_slice(b"G701F");
    report(pipe.ingest(&other, now_ns()));

    println!("a truncated B601F is refused by the length check before any field is read");
    report(pipe.ingest(&BOOK[..BOOK.len() - 1], now_ns()));

    println!("a B601F with a corrupted price fails in the decoder, and nothing is published");
    let mut bad = BOOK.to_vec();
    bad[47] = b'X'; // first byte of 매도호가1
    report(pipe.ingest(&bad, now_ns()));

    let stats = pipe.stats();
    println!(
        "\nreceived {}  published {}  filtered {}  wrong length {}  failed {}",
        stats.received, stats.published, stats.filtered_trcode, stats.wrong_length, stats.decode_failed
    );
}

fn report(outcome: Outcome) {
    match outcome {
        Outcome::Published { trcode } => println!("    published (filter slot {trcode})"),
        other => println!("    {other:?}"),
    }
}
