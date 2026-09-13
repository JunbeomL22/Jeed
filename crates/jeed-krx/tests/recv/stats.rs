//! `jeed_krx::recv::stats`.

use jeed_krx::TrCode;
use jeed_krx::recv::{Endpoint, SocketStats, Stats, TrCodeFilter};

fn code(s: &str) -> TrCode {
    TrCode::from_message(s.as_bytes()).unwrap()
}

fn endpoint() -> Endpoint {
    "233.38.231.92:10302".parse().unwrap()
}

#[test]
fn filtered_traffic_is_not_loss() {
    // The two numbers answer different questions: "is the wiring right?" and
    // "did we lose anything we wanted?". Adding them would make the second
    // unanswerable on a shared port.
    let s = Stats {
        filtered_trcode: 1_000_000,
        filtered_isin: 5_000,
        decode_failed: 2,
        wrong_length: 1,
        ..Stats::default()
    };
    assert_eq!(s.filtered(), 1_005_000);
    assert_eq!(s.dropped(), 3);
}

#[test]
fn a_socket_reports_which_configured_codes_it_has_never_carried() {
    // Startup cannot check that a port carries what conf says it does — the
    // assignment is a circuit matter. This is the runtime substitute.
    let filter = TrCodeFilter::new(["B601F", "G701F", "V101F"].map(code));
    let mut ch = SocketStats::new(endpoint(), &filter);

    ch.mark_seen(filter.index_of(code("B601F")).unwrap());
    ch.mark_seen(filter.index_of(code("G701F")).unwrap());

    let unseen: Vec<_> = ch.never_seen(&filter).collect();
    assert_eq!(unseen, vec![code("V101F")]);
}

#[test]
fn a_socket_that_has_carried_nothing_reports_every_code() {
    let filter = TrCodeFilter::new(["B601F", "G701F"].map(code));
    let ch = SocketStats::new(endpoint(), &filter);
    assert_eq!(ch.never_seen(&filter).count(), 2);
    assert_eq!(ch.received, 0);
    assert_eq!(ch.last_recv_ns, 0, "a quiet socket and a dead one look the same here");
}

#[test]
fn marking_a_code_the_filter_does_not_hold_is_ignored() {
    let filter = TrCodeFilter::new([code("B601F")]);
    let mut ch = SocketStats::new(endpoint(), &filter);
    ch.mark_seen(99);
    assert!(!ch.saw(99));
    assert_eq!(ch.never_seen(&filter).count(), 1);
}
