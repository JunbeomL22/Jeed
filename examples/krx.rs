//! The whole KRX pipeline in one process: conf → ring → socket → feed thread →
//! a datagram on loopback multicast → the record a consumer reads off the ring.
//!
//! ```text
//! cargo run -p jeed --example krx
//! ```
//!
//! No circuit is needed. The feed joins an administratively scoped group on
//! `127.0.0.1`, and this process sends it the two datagrams embedded below — a
//! KOSPI200 futures book (`B601F`) and a print (`A301F`), built from the
//! interface definitions exactly as the decoder tests build them.
//!
//! What runs is what `jeed-krx conf/krx.toml --no-pin` runs: the same conf
//! reader, the same `start`, the same receive loop and ring. Only the conf is
//! a string instead of a file, and the core pin is off. The consumer half is
//! what an OMS process does: `jeed-wire` + `jeed-shm`, attach at the live
//! edge, read.

use core::sync::atomic::{AtomicBool, Ordering};
use jeed::conf::KrxConf;
use jeed::krx::{Options, start};
use jeed::toml::parse;
use jeed_krx::recv::IsinFilter;
use jeed_shm::{Recv, RingConsumer, SegmentName, SharedMapping};
use jeed_wire::{WireKind, WireRecord, trade_kind};
use std::net::UdpSocket;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One feed, one ring. Everything a real conf has except the core pin, which
/// `Options::pin = false` turns off below. `@127.0.0.1` joins on loopback —
/// and on Windows keeps the firewall prompt away.
const CONF: &str = r#"
[[feed]]
name = "hot"
mode = "spin"
cores = [0]
ring = "jeed.example.krx"
ring_slots = 64
sockets = ["239.255.79.42:31912@127.0.0.1"]
trcodes = ["B601F", "A301F"]

[health]
heartbeat_ms = 100
stale_ms = 500
"#;

const RING: &str = "jeed.example.krx";
const GROUP: &str = "239.255.79.42:31912";

/// `B601F` 우선호가 — KOSPI200 선물 `KR4101V90009`, five levels, best 937.05 / 936.95.
const BOOK: &[u8] = b"B601F00000001G140KR4101V90009000001090100123456000937.05000936.950000000100000000080000300002000937.10000936.900000000250000000310000700009000937.15000936.850000000400000000440001100012000937.20000936.800000000550000000600001500016000937.25000936.7500000007000000007700019000200000000000000000000000000000000000.00000000000\xff";

/// `A301F` 체결 — the same contract printing 3 lots at 937.00.
const TRADE: &[u8] = b"A301F00000001G140KR4101V90009000001090100123456000937.00000000003000000.00000000.00000935.10000938.40000933.75000936.95000000166478000000000155908543.0002000946.35000927.65\xff";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. conf — the same reader and the same rules as the binary.
    let conf = KrxConf::from_table(&parse(CONF)?)?;
    conf.validate(None)?;

    // 2. start — ring created, socket joined, thread running. Anything that
    //    can fail has failed before this returns.
    let stop = Arc::new(AtomicBool::new(false));
    let opts = Options { boot_id: jeed::boot_id(), pin: false, report_ns: 0 };
    let feeds = start(&conf, &IsinFilter::all(), opts, Arc::clone(&stop))?;
    println!("started {} feed(s): {}", feeds.len(), feeds.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", "));

    // 3. the consumer side — attach at the live edge.
    let name = SegmentName::local(RING)?;
    let mut rx = RingConsumer::attach(&name)?;
    println!("consumer attached to `{RING}`: {} slots, boot_id {:#x}", rx.capacity(), rx.boot_id());

    // 4. play the circuit: two datagrams to the group, from loopback.
    let sender = UdpSocket::bind("127.0.0.1:0")?;
    sender.set_multicast_loop_v4(true)?;
    sender.send_to(BOOK, GROUP)?;
    sender.send_to(TRADE, GROUP)?;
    println!("sent B601F ({} B) and A301F ({} B) to {GROUP}", BOOK.len(), TRADE.len());

    // 5. read what comes off the ring: the two records, and heartbeats.
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut rec = WireRecord::zeroed();
    let (mut quotes, mut trades, mut heartbeats) = (0, 0, 0);
    while Instant::now() < deadline && (quotes == 0 || trades == 0) {
        match rx.try_recv(&mut rec) {
            Recv::Record => {
                match rec.kind() {
                    Ok(WireKind::Quote) => quotes += 1,
                    Ok(WireKind::Trade) => trades += 1,
                    Ok(WireKind::Heartbeat) => heartbeats += 1,
                    _ => {}
                }
                println!("  {}", describe(&rec));
            }
            Recv::Lagged(n) => println!("  lagged: {n} lost"),
            Recv::Restarted { boot_id } => println!("  producer restarted: {boot_id:#x}"),
            Recv::Empty => std::thread::sleep(Duration::from_millis(1)),
        }
    }

    // 6. stop, as Ctrl-C would.
    stop.store(true, Ordering::Relaxed);
    for feed in feeds {
        let name = feed.name.clone();
        feed.join().map_err(|e| format!("{name}: {e}"))?;
    }
    println!("stopped. quotes {quotes}, trades {trades}, heartbeats {heartbeats}");

    // POSIX names outlive the process; on Windows the section dies with the
    // last handle and this is a no-op.
    drop(rx);
    SharedMapping::unlink(&name)?;

    if quotes == 0 || trades == 0 {
        eprintln!("the datagrams did not come back: multicast loopback is unavailable on this host");
        std::process::exit(1);
    }
    Ok(())
}

fn describe(rec: &WireRecord) -> String {
    let h = &rec.header;
    let scale = h.price_scale().map_or(1.0, |s| s.multiplier());
    let symbol = String::from_utf8_lossy(h.symbol_bytes());
    let head = format!("seq {:>3}", h.producer_seq);
    match rec.kind() {
        Ok(WireKind::Quote) => {
            let q = rec.quote().unwrap();
            let bid = q.bids(h.depth)[0];
            let ask = q.asks(h.depth)[0];
            format!(
                "{head}  Quote      {symbol}  bid {:.2} x {}  ask {:.2} x {}  depth {}",
                bid.price as f64 * scale,
                bid.qty,
                ask.price as f64 * scale,
                ask.qty,
                h.depth
            )
        }
        Ok(WireKind::Trade) => {
            let t = rec.trade().unwrap();
            let side = match t.trade_kind {
                trade_kind::BUY => "buyer aggressed",
                trade_kind::SELL => "seller aggressed",
                _ => "aggressor unknown",
            };
            format!("{head}  Trade      {symbol}  {:.2} x {}  ({side})", t.price as f64 * scale, t.qty)
        }
        Ok(WireKind::Heartbeat) => {
            let hb = rec.heartbeat().unwrap();
            format!("{head}  Heartbeat  received {} forwarded {}", hb.received, hb.forwarded)
        }
        Ok(kind) => format!("{head}  {kind:?}"),
        Err(e) => format!("{head}  <invalid: {e}>"),
    }
}
