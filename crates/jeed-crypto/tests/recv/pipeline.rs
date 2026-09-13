//! Tests for `src/recv/pipeline.rs` — route, publish, stale, heartbeat,
//! with no socket anywhere.

use crate::frames::{RECV_NS, SPOT_TRADE};
use crate::router::Fake;
use crate::sink::Collect;
use jeed_crypto::CryptoError;
use jeed_crypto::recv::{Outcome, Pipeline};
use jeed_wire::{Venue, WireKind, header_flags};

fn pipeline(stale_ns: u64) -> Pipeline<Fake, Collect> {
    Pipeline::new(stale_ns, Fake::new(), Collect::new())
}

#[test]
fn a_trade_message_is_routed_and_published() {
    let mut p = pipeline(0);
    let out = p.ingest(SPOT_TRADE, RECV_NS);

    assert_eq!(out, Outcome::Published { records: 1 });
    assert!(out.published());
    assert_eq!(p.sink().len(), 1);
    assert_eq!(p.sink().last().header.kind, WireKind::Trade.as_u8());
    assert_eq!(p.stats().messages, 1);
    assert_eq!(p.stats().published, 1);
}

#[test]
fn zero_records_is_published_and_not_a_failure() {
    let mut p = pipeline(0);
    let out = p.ingest(br#"{"result":null,"id":1}"#, RECV_NS);

    assert_eq!(out, Outcome::Published { records: 0 });
    assert!(!out.published());
    assert_eq!(p.stats().decode_failed, 0);
    assert_eq!(p.sink().drops, 0);
}

#[test]
fn a_refused_message_counts_and_notes_a_drop() {
    let mut p = pipeline(0);
    let out = p.ingest(br#"{"e":"trade","s":"ETHUSDT"}"#, RECV_NS);

    assert!(matches!(out, Outcome::Failed(CryptoError::SymbolMismatch)));
    assert_eq!(p.sink().len(), 0, "nothing was published");
    assert_eq!(p.sink().drops, 1);
    assert_eq!(p.stats().decode_failed, 1);
    assert_eq!(p.stats().dropped(), 1);
}

#[test]
fn a_late_record_is_flagged_stale_and_counted() {
    // SPOT_TRADE's `T` is 1755088771744 ms; RECV_NS is ~28 s later.
    let mut p = pipeline(1_000_000_000);
    p.ingest(SPOT_TRADE, RECV_NS);

    let rec = p.sink().last();
    assert!(rec.header.flags & header_flags::VENUE_TIME_VALID != 0, "the trade carries a venue time");
    assert!(rec.header.flags & header_flags::STALE != 0);
    assert_eq!(p.stats().stale, 1);
}

#[test]
fn a_fresh_record_is_not_stale() {
    let mut p = pipeline(60_000_000_000);
    p.ingest(SPOT_TRADE, RECV_NS);

    assert!(p.sink().last().header.flags & header_flags::STALE == 0);
    assert_eq!(p.stats().stale, 0);
}

#[test]
fn stale_zero_disables_the_check() {
    let mut p = pipeline(0);
    p.ingest(SPOT_TRADE, RECV_NS + 3_600_000_000_000);

    assert!(p.sink().last().header.flags & header_flags::STALE == 0);
}

#[test]
fn skew_reads_as_age_zero() {
    // recv before the venue time: the clocks disagree, the record is not old.
    let mut p = pipeline(1);
    p.ingest(SPOT_TRADE, 1_000_000_000_000_000_000);

    assert!(p.sink().last().header.flags & header_flags::STALE == 0);
}

#[test]
fn the_heartbeat_record_carries_the_counters() {
    let mut p = pipeline(0);
    p.ingest(SPOT_TRADE, RECV_NS);
    p.ingest(br#"{"result":null,"id":1}"#, RECV_NS);
    p.ingest(b"junk", RECV_NS);
    p.heartbeat(RECV_NS + 5);

    let rec = p.sink().last();
    assert_eq!(rec.header.kind, WireKind::Heartbeat.as_u8());
    assert_eq!(rec.header.venue, Venue::BinanceSpot.as_u8());
    assert_eq!(rec.header.recv_ns, RECV_NS + 5);
    let hb = rec.heartbeat().expect("a heartbeat payload");
    assert_eq!(hb.received, 3, "messages, not records");
    assert_eq!(hb.forwarded, 1);
    assert_eq!(p.stats().heartbeats, 1);
}

#[test]
fn counters_are_plain_increments() {
    let mut p = pipeline(0);
    p.note_bytes(10);
    p.note_bytes(5);
    p.note_frame();
    p.note_fragment();
    p.note_binary();
    p.note_ping();
    p.note_pong();
    p.note_close();
    p.note_ping_sent();
    p.note_pong_sent();
    p.note_keepalive();
    p.note_protocol_error();
    p.note_disconnect();
    p.note_connect_failure();
    p.note_open();

    let s = p.stats();
    assert_eq!(s.bytes, 15);
    assert_eq!(
        [
            s.frames, s.fragments, s.binary, s.pings, s.pongs, s.closes, s.pings_sent, s.pongs_sent,
            s.keepalives_sent, s.protocol_errors, s.disconnects, s.connect_failures, s.opens
        ],
        [1; 13]
    );
    assert_eq!(s.dropped(), 0, "binary frames are not drops");
}

#[test]
fn the_sink_comes_back() {
    let mut p = pipeline(0);
    p.ingest(SPOT_TRADE, RECV_NS);
    assert_eq!(p.into_sink().len(), 1);
}
