//! `jeed::krx` — the wiring, end to end: conf → rings → sockets → threads →
//! a datagram on the group → a record a consumer reads off the ring.
//!
//! The groups are administratively scoped (`239.255.0.0/16`) and joined,
//! sent and looped back on `127.0.0.1` only — a UDP socket on `INADDR_ANY`
//! raises the Windows firewall prompt for every fresh test binary, and a
//! managed box cannot answer it. The 전문 is the decoder tests' own builder,
//! reached by path rather than copied (`CLAUDE.md`).

#[allow(unused_imports)]
#[path = "../../jeed-krx/tests/decode/common/mod.rs"]
mod builders;

use builders::{B6, kospi200_book};
use core::sync::atomic::{AtomicBool, Ordering};
use jeed::conf::KrxConf;
use jeed::krx::{Options, start};
use jeed::toml::parse;
use jeed_krx::recv::IsinFilter;
use jeed_shm::{Recv, RingConsumer, SegmentName, SharedMapping};
use jeed_wire::{Venue, WireKind, WireRecord};
use std::net::UdpSocket;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn conf(pid: u32) -> KrxConf {
    let text = format!(
        r#"
        [[feed]]
        name = "hot"
        mode = "spin"
        cores = [0]
        ring = "jeed.test.{pid}.hot"
        ring_slots = 64
        sockets = ["239.255.79.92:31902@127.0.0.1", "239.255.79.96:31922@127.0.0.1"]
        trcodes = ["B601F", "G701F"]

        [[feed]]
        name = "cold"
        mode = "block"
        cores = [1]
        ring = "jeed.test.{pid}.cold"
        ring_slots = 16
        sockets = ["239.255.79.93:31915@127.0.0.1"]
        trcodes = ["M401F"]

        [health]
        heartbeat_ms = 20
        stale_ms = 500
        "#
    );
    KrxConf::from_table(&parse(&text).unwrap()).unwrap()
}

/// Reads everything published since `rx` attached (or last drained).
fn drain(rx: &mut RingConsumer) -> Vec<WireRecord> {
    let mut out = Vec::new();
    let mut rec = WireRecord::zeroed();
    loop {
        match rx.try_recv(&mut rec) {
            Recv::Record => out.push(rec),
            Recv::Empty => return out,
            Recv::Lagged(_) | Recv::Restarted { .. } => {}
        }
    }
}

#[test]
fn two_feeds_start_publish_and_stop() {
    let pid = std::process::id();
    let conf = conf(pid);
    conf.validate(None).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let opts = Options { boot_id: 0xfeed_0000_0000_0001 | u64::from(pid), pin: false, report_ns: 0 };
    let feeds = start(&conf, &IsinFilter::all(), opts, Arc::clone(&stop)).unwrap();
    assert_eq!(feeds.len(), 2);
    assert_eq!(feeds[0].name, "hot");
    assert_eq!(feeds[1].name, "cold");

    // A 코스피200 선물 book on the hot feed's first group.
    // Bound to loopback, the datagram leaves on loopback (measured; no
    // `IP_MULTICAST_IF` needed), which is where the feeds joined.
    let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
    sender.set_multicast_loop_v4(true).unwrap();
    let msg = B6::kospi200(kospi200_book()).build();
    let hot = SegmentName::local(&format!("jeed.test.{pid}.hot")).unwrap();
    let cold = SegmentName::local(&format!("jeed.test.{pid}.cold")).unwrap();

    // Both rings carry this run's identity. Attach now: a consumer starts at
    // the ring's end and sees only what is published after it attached.
    let mut hot_rx = RingConsumer::attach(&hot).unwrap();
    assert_eq!(hot_rx.header().boot_id, opts.boot_id);
    let mut cold_rx = RingConsumer::attach(&cold).unwrap();
    assert_eq!(cold_rx.header().boot_id, opts.boot_id);

    // Loopback multicast is best-effort: send until the ring shows a quote,
    // or give up after a while.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut published = Vec::new();
    while Instant::now() < deadline {
        sender.send_to(&msg, "239.255.79.92:31902").unwrap();
        std::thread::sleep(Duration::from_millis(50));
        published.extend(drain(&mut hot_rx));
        if published.iter().any(|r| r.kind() == Ok(WireKind::Quote)) {
            break;
        }
    }

    stop.store(true, Ordering::Relaxed);
    for feed in feeds {
        let name = feed.name.clone();
        feed.join().unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    published.extend(drain(&mut hot_rx));

    let cold_records = drain(&mut cold_rx);
    assert!(!cold_records.is_empty(), "the cold feed heartbeats even with nothing to receive");
    assert!(
        cold_records.iter().all(|r| r.kind() == Ok(WireKind::Heartbeat)),
        "the cold feed received nothing, so its ring is heartbeats only"
    );
    assert_eq!(hot_rx.header().capacity, 64);
    assert_eq!(cold_rx.header().capacity, 16);

    let quotes: Vec<&WireRecord> = published.iter().filter(|r| r.kind() == Ok(WireKind::Quote)).collect();
    if quotes.is_empty() {
        // The loopback route was not there. The threads still started,
        // heartbeat, and stopped cleanly, which is what this test is for; the
        // decode path has its own tests in `jeed-krx`.
        eprintln!("no looped-back datagram arrived; multicast loopback is unavailable on this host");
        assert!(published.iter().any(|r| r.kind() == Ok(WireKind::Heartbeat)));
    } else {
        let q = quotes[0];
        assert_eq!(q.header.venue, Venue::Krx as u8);
        assert_eq!(&q.header.symbol[..12], b"KR4101V90009");
        assert!(q.header.producer_seq < 64);
    }

    // POSIX names outlive the process; Windows sections die with the last
    // handle and `unlink` is a no-op there.
    drop(hot_rx);
    drop(cold_rx);
    SharedMapping::unlink(&hot).unwrap();
    SharedMapping::unlink(&cold).unwrap();
}

#[test]
fn a_bad_group_starts_nothing() {
    // The second feed's socket cannot be joined, so the first feed's ring —
    // already created — is dropped again and no thread runs.
    let pid = std::process::id();
    let mut conf = conf(pid);
    conf.feeds[1].sockets[0].group = std::net::Ipv4Addr::new(10, 1, 2, 3);
    for feed in &mut conf.feeds {
        feed.ring = format!("{}.bad", feed.ring);
    }

    let stop = Arc::new(AtomicBool::new(false));
    let opts = Options { boot_id: 7, pin: false, report_ns: 0 };
    let err = start(&conf, &IsinFilter::all(), opts, stop).unwrap_err();
    assert!(matches!(&err, jeed::krx::StartError::Net { feed, .. } if feed == "cold"), "{err}");

    let hot = SegmentName::local(&format!("jeed.test.{pid}.hot.bad")).unwrap();
    let _ = SharedMapping::unlink(&hot);
}
