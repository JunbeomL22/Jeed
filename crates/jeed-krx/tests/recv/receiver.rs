//! `jeed_krx::recv::receiver` — the socket loop.

use crate::sink::Collect;
use jeed_krx::recv::{Config, Endpoint, IsinFilter, MAX_DATAGRAM, Mode, Receiver, TrCodeFilter};
use jeed_krx::{TrCode, decode::dispatch};
use jeed_wire::WireKind;
use std::cell::Cell;
use std::net::Ipv4Addr;

fn code(s: &str) -> TrCode {
    TrCode::from_message(s.as_bytes()).unwrap()
}

/// A receiver with no sockets: the loop skeleton with nothing to drain.
fn quiet(cfg: Config) -> Receiver<Collect> {
    Receiver::new(cfg, &[], TrCodeFilter::new([code("B601F")]), IsinFilter::all(), Collect::new())
        .unwrap()
}

#[test]
fn the_read_buffer_holds_the_longest_message_krx_defines() {
    // 1387 B REPO 우선호가 is the longest interface in the standard. A buffer
    // under it would truncate that channel, and the length check would then
    // reject every message on it, one at a time, forever.
    const { assert!(MAX_DATAGRAM >= 1387) };
    let longest = ["B601F", "G704F", "B703S", "M401F"]
        .into_iter()
        .filter_map(|c| dispatch::message_len(code(c)))
        .max()
        .unwrap();
    assert!(MAX_DATAGRAM > longest);
}

#[test]
fn the_poll_timeout_is_the_heartbeat_interval() {
    // The cold feed tick and the liveness cadence are one thing. A second knob
    // could only put them out of step.
    let cfg = Config { heartbeat_ns: 100_000_000, ..Config::default() };
    assert_eq!(cfg.poll_timeout_ms(), 100);
}

#[test]
fn the_poll_timeout_is_clamped_at_both_ends() {
    let fast = Config { heartbeat_ns: 1_000, ..Config::default() };
    assert_eq!(fast.poll_timeout_ms(), 1, "never a zero-timeout spin in block mode");

    let slow = Config { heartbeat_ns: 60_000_000_000, ..Config::default() };
    assert_eq!(slow.poll_timeout_ms(), 1000, "still wakes often enough to notice a stop request");

    let off = Config { heartbeat_ns: 0, ..Config::default() };
    assert_eq!(off.poll_timeout_ms(), 100, "no heartbeat still needs a tick");
}

#[test]
fn the_defaults_have_both_guards_switched_on() {
    // Not a style preference: a conf that omitted the stale threshold once left
    // a book frozen for twenty minutes, so the omission has to fail safe.
    let cfg = Config::default();
    assert_ne!(cfg.heartbeat_ns, 0);
    assert_ne!(cfg.stale_ns, 0);
    assert_eq!(cfg.mode, Mode::Spin);
}

#[test]
fn the_first_round_says_the_producer_is_alive() {
    // Before any market data has arrived to say it implicitly.
    let mut rx = quiet(Config { mode: Mode::Block, ..Config::default() });
    rx.poll_once().unwrap();

    assert_eq!(rx.stats().heartbeats, 1);
    let sink = rx.into_sink();
    assert_eq!(sink.records.len(), 1);
    assert_eq!(sink.records[0].kind().unwrap(), WireKind::Heartbeat);
}

#[test]
fn the_next_one_waits_for_the_interval() {
    let mut rx = quiet(Config { mode: Mode::Block, ..Config::default() });
    rx.poll_once().unwrap();
    rx.poll_once().unwrap();
    assert_eq!(rx.stats().heartbeats, 1, "two rounds inside 100 ms is one heartbeat");
}

#[test]
fn a_zero_interval_turns_the_heartbeat_off() {
    let mut rx = quiet(Config { mode: Mode::Block, heartbeat_ns: 0, ..Config::default() });
    for _ in 0..3 {
        rx.poll_once().unwrap();
    }
    assert_eq!(rx.stats().heartbeats, 0);
}

#[test]
fn run_until_stops_when_asked() {
    let rounds = Cell::new(0u32);
    let mut rx = quiet(Config { mode: Mode::Block, heartbeat_ns: 0, ..Config::default() });
    rx.run_until(|| {
        let n = rounds.get();
        rounds.set(n + 1);
        n >= 5
    })
    .unwrap();
    assert_eq!(rounds.get(), 6);
}

#[test]
fn every_endpoint_gets_its_own_counters_in_the_order_given() {
    let endpoints = [
        Endpoint::new(Ipv4Addr::new(239, 255, 77, 90), 30_890).on(Ipv4Addr::LOCALHOST),
        Endpoint::new(Ipv4Addr::new(239, 255, 77, 91), 30_891).on(Ipv4Addr::LOCALHOST),
    ];
    let filter = TrCodeFilter::new(["B601F", "G701F"].map(code));
    let rx = Receiver::new(
        Config::default(),
        &endpoints,
        filter.clone(),
        IsinFilter::all(),
        Collect::new(),
    )
    .unwrap();

    assert_eq!(rx.channels().len(), 2);
    for (channel, endpoint) in rx.channels().iter().zip(endpoints) {
        assert_eq!(channel.endpoint, endpoint);
        assert_eq!(channel.received, 0);
        // Nothing has arrived, so every configured code is still unaccounted
        // for — which is also what a mis-assigned port looks like.
        assert_eq!(channel.never_seen(&filter).count(), filter.len());
    }
}

#[test]
fn two_endpoints_on_one_port_are_refused() {
    // A Windows multicast receiver binds to the interface, not the group, so
    // both sockets would receive both groups and every message would be
    // published twice. A doubled book is not something to discover from the
    // ring.
    let clash = [
        Endpoint::new(Ipv4Addr::new(239, 255, 77, 93), 30_893).on(Ipv4Addr::LOCALHOST),
        Endpoint::new(Ipv4Addr::new(239, 255, 77, 94), 30_893).on(Ipv4Addr::LOCALHOST),
    ];
    let err = Receiver::new(
        Config::default(),
        &clash,
        TrCodeFilter::new([code("B601F")]),
        IsinFilter::all(),
        Collect::new(),
    )
    .unwrap_err();

    assert_eq!(err.call, "duplicate port");
    assert_eq!(err.endpoint.port, 30_893);
}

#[test]
fn a_unicast_endpoint_fails_the_join_rather_than_the_market_open() {
    let bad = [Endpoint::new(Ipv4Addr::new(10, 20, 30, 40), 30_892)];
    let out = Receiver::new(
        Config::default(),
        &bad,
        TrCodeFilter::new([code("B601F")]),
        IsinFilter::all(),
        Collect::new(),
    );
    assert!(out.is_err());
}
