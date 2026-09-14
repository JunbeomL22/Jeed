//! Producer and consumer in one process — the ring's three answers, end to end.
//!
//! Nothing external is needed. Run it with:
//!
//! ```text
//! cargo run -p jeed-shm --example roundtrip
//! ```
//!
//! It creates a tiny ring, publishes a few quotes, reads them back, then
//! overruns the consumer on purpose to show `Recv::Lagged` and the *live edge*
//! rule, and finally restarts the producer to show `Recv::Restarted`.

use jeed_shm::{Recv, RingConsumer, RingProducer, SegmentName};
use jeed_wire::{
    QuotePayload, RecordHeader, RecordSink, Scale, UnixNano, Venue, WireKind, WireLevel, WireRecord,
    symbol_from_bytes,
};
use std::time::{SystemTime, UNIX_EPOCH};

const RING: &str = "jeed.example.roundtrip";
const CAPACITY: u64 = 8; // power of two — the slot index is a mask, not a division

fn now_ns() -> UnixNano {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64)
}

/// A KOSPI200 futures book two levels deep, prices at 2 decimals (937.05 → 93705).
fn fill_quote(rec: &mut WireRecord, best_bid: i64, seq_hint: u64) {
    let symbol = symbol_from_bytes(b"KR4101V90009").expect("12 bytes fits");
    let mut header = RecordHeader::new(WireKind::Quote, Venue::Krx, symbol, now_ns());
    header.set_scales(Scale::from_decimals(2).unwrap(), Scale::from_decimals(0).unwrap()).set_depth(2);

    let mut quote = QuotePayload::default();
    quote
        .set_bid(0, WireLevel::with_count(best_bid, 10 + seq_hint, 3))
        .set_bid(1, WireLevel::with_count(best_bid - 5, 25, 7))
        .set_ask(0, WireLevel::with_count(best_bid + 10, 8, 2))
        .set_ask(1, WireLevel::with_count(best_bid + 15, 31, 7))
        .with_order_counts();

    *rec = WireRecord::new_quote(header, quote);
}

fn describe(rec: &WireRecord) -> String {
    let h = &rec.header;
    let symbol = String::from_utf8_lossy(h.symbol_bytes()).into_owned();
    match rec.kind() {
        Ok(WireKind::Quote) => {
            let q = rec.quote().unwrap();
            let scale = h.price_scale().unwrap().multiplier();
            let bid = q.bids(h.depth)[0];
            let ask = q.asks(h.depth)[0];
            format!(
                "seq {:>3}  Quote  {symbol}  bid {:.2} x {}  ask {:.2} x {}  depth {}",
                h.producer_seq,
                bid.price as f64 * scale,
                bid.qty,
                ask.price as f64 * scale,
                ask.qty,
                h.depth
            )
        }
        Ok(kind) => format!("seq {:>3}  {kind:?}  {symbol}", h.producer_seq),
        Err(e) => format!("seq {:>3}  <invalid kind: {e}>", h.producer_seq),
    }
}

fn drain(rx: &mut RingConsumer) {
    let mut rec = WireRecord::zeroed();
    loop {
        match rx.try_recv(&mut rec) {
            Recv::Record => println!("    {}", describe(&rec)),
            Recv::Lagged(n) => println!("    Lagged({n}) — {n} records were overwritten; the cursor is at the live edge"),
            Recv::Restarted { boot_id } => println!("    Restarted {{ boot_id: {boot_id:#x} }} — everything built so far is stale"),
            Recv::Empty => return,
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let name = SegmentName::local(RING)?;

    println!("1. create a {CAPACITY}-slot ring and attach a consumer at its live edge");
    let mut tx = RingProducer::create(&name, CAPACITY, 0x1001)?;
    let mut rx = RingConsumer::attach(&name)?;
    println!("    capacity {}  boot_id {:#x}  published so far {}", rx.capacity(), rx.boot_id(), rx.published());

    println!("2. publish three quotes through the `RecordSink` trait — the same call the decoders make");
    for i in 0..3u64 {
        // `publish` claims a slot, runs the closure, and commits only on `Ok`.
        // A decoder that fails part-way leaves nothing on the ring.
        tx.publish(|rec| -> Result<(), ()> {
            fill_quote(rec, 93_700 + i as i64 * 5, i);
            Ok(())
        })
        .unwrap();
    }
    drain(&mut rx);

    println!("3. a failed fill publishes nothing");
    let refused: Result<(), &str> = tx.publish(|rec| {
        rec.header.depth = 99; // half-written …
        Err("decoder gave up") // … and abandoned: the slot is never committed
    });
    println!("    publish returned {refused:?}; consumer sees:");
    drain(&mut rx);

    println!("4. overrun the consumer: {} records into a {CAPACITY}-slot ring without reading", CAPACITY * 2 + 3);
    for i in 0..(CAPACITY * 2 + 3) {
        let mut rec = WireRecord::zeroed();
        fill_quote(&mut rec, 93_800 + i as i64, i);
        tx.push(&rec);
    }
    println!("    the consumer reports the loss, then reads from the live edge — it never replays a stale lap:");
    drain(&mut rx);

    println!("5. restart the producer with a new boot_id");
    drop(tx);
    let mut tx = RingProducer::create(&name, CAPACITY, 0x1002)?;
    println!("    reused the existing section: {}", tx.reused_existing_section());
    let mut rec = WireRecord::zeroed();
    fill_quote(&mut rec, 94_000, 0);
    tx.push(&rec);
    drain(&mut rx);
    println!("    the cursor is re-seated at the new run's live edge, so that first record is behind it; the next one is read:");
    fill_quote(&mut rec, 94_005, 1);
    tx.push(&rec);
    drain(&mut rx);

    Ok(())
}
