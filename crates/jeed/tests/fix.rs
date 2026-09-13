//! `jeed::fix` — the conf-driven venue adapter, driven by the same synthetic
//! SMBS messages the `jeed-fix` crate tests its decoder on.
//!
//! The builders are reached by `#[path]` rather than copied, the way the KRX
//! and crypto end-to-end tests reach theirs (`CLAUDE.md`). The end-to-end
//! session — connect, logon, subscribe — needs a FIX peer and is out of reach
//! here; what these pin is the step this crate actually owns: a decoded
//! message becoming the right wire record, or the right refusal.

#[allow(dead_code)]
#[path = "../../jeed-fix/tests/fix/builders.rs"]
mod builders;

use builders::{OPEN_SNAPSHOT, SCALES, TRADE_INCREMENTAL, TWO_SIDED_SNAPSHOT, wrap};
use jeed::fix::{AdaptError, SnapshotAdapter};
use jeed_fix::recv::MdAdapter;
use jeed_fix::{FixScales, MdMessage, frame, parse_md_message};
use jeed_wire::{RecordSink, Scale, Venue, WireKind, WireRecord, header_flags};

const RECV_NS: u64 = 1_769_990_400_500_000_000;

/// Collects published records.
#[derive(Default)]
struct Sink {
    records: Vec<WireRecord>,
    drops: u64,
}

impl RecordSink for Sink {
    fn publish<E>(&mut self, fill: impl FnOnce(&mut WireRecord) -> Result<(), E>) -> Result<(), E> {
        let mut rec = WireRecord::zeroed();
        fill(&mut rec)?;
        self.records.push(rec);
        Ok(())
    }
    fn note_drops(&mut self, n: u64) {
        self.drops += n;
    }
}

fn decode(body: &str) -> MdMessage {
    let raw = wrap(body);
    let f = frame(&raw).expect("frames");
    parse_md_message(&f, SCALES).expect("decodes")
}

fn adapt(body: &str) -> (Result<usize, AdaptError>, Sink) {
    let mut adapter = SnapshotAdapter::new(Venue::Smbs, SCALES, None);
    let msg = decode(body);
    let mut sink = Sink::default();
    let r = adapter.adapt(&msg, RECV_NS, &mut sink);
    (r, sink)
}

#[test]
fn a_two_sided_snapshot_becomes_one_quote() {
    let (r, sink) = adapt(TWO_SIDED_SNAPSHOT);
    assert_eq!(r, Ok(1));
    assert_eq!(sink.records.len(), 1);
    let rec = &sink.records[0];
    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.venue, Venue::Smbs.as_u8());
    assert_eq!(rec.header.symbol_bytes(), b"USDKRW");
    assert_eq!(rec.header.depth, 1);
    // Scales are the session's, and the price is the integer on that scale.
    assert_eq!(rec.header.price_scale(), Ok(Scale::S2));
    let q = rec.quote().unwrap();
    assert_eq!(q.bid[0].price, 145000, "1450.00 × 100");
    assert_eq!(q.ask[0].price, 145300, "1453.00 × 100");
    assert_eq!(q.bid[0].qty, 5_000_000);
    assert_eq!(q.ask[0].qty, 5_000_000);
    assert_eq!(rec.header.flags & (header_flags::BID_EMPTY | header_flags::ASK_EMPTY), 0);
    assert_eq!(rec.validate(), Ok(()));
}

#[test]
fn a_one_sided_open_snapshot_flags_the_empty_side() {
    // The open book carries only an offer (269=1); the bid side is empty and
    // the consumer must be told, not left to read a zero as a price.
    let (r, sink) = adapt(OPEN_SNAPSHOT);
    assert_eq!(r, Ok(1));
    let rec = &sink.records[0];
    let q = rec.quote().unwrap();
    assert_eq!(q.ask[0].price, 145300);
    assert_eq!(q.bid[0].price, 0);
    assert!(rec.header.has(header_flags::BID_EMPTY), "no bid side");
    assert!(!rec.header.has(header_flags::ASK_EMPTY), "offer present");
}

#[test]
fn an_incremental_trade_becomes_a_trade() {
    let (r, sink) = adapt(TRADE_INCREMENTAL);
    assert_eq!(r, Ok(1));
    let rec = &sink.records[0];
    assert_eq!(rec.kind(), Ok(WireKind::Trade));
    assert_eq!(rec.header.symbol_bytes(), b"USDKRW");
    let t = rec.trade().unwrap();
    assert_eq!(t.price, 145100, "1451.00 × 100");
    assert_eq!(t.qty, 1_000_000);
}

#[test]
fn a_venue_time_rides_the_record() {
    let (_r, sink) = adapt(TWO_SIDED_SNAPSHOT);
    // 52/272-273 give the venue's own clock; the adapter carries it so the
    // stale guard can compare it against recv_ns.
    assert!(sink.records[0].header.venue_time().is_some());
}

#[test]
fn a_book_moving_incremental_is_refused_not_dropped() {
    // 269=0 (a bid) on a 35=X moves the book, and the wire has no delta record
    // for it. Refusing lets the consumer resynchronise; publishing a trade or
    // dropping it silently would leave the book wrong.
    let book_delta = "35=X|49=SMBS|56=FRACTAL|34=19|52=20260202-00:00:15.005|\
         262=USDKRW-SMBS|268=1|279=0|269=0|55=USDKRW|270=1450.50|271=3000000|\
         272=20260202|273=00:00:15.000";
    let (r, sink) = adapt(book_delta);
    assert_eq!(r, Err(AdaptError::IncrementalBook));
    assert!(sink.records.is_empty(), "nothing published on a refusal");
}

#[test]
fn a_snapshot_level_with_no_size_is_refused_unless_a_default_is_set() {
    // 271 (MDEntrySize) absent. With no default_qty the adapter refuses rather
    // than invent a size; with one it publishes that size.
    let no_size = "35=W|49=SMBS|56=FRACTAL|34=3|52=20260202-00:00:02.005|\
         262=USDKRW-SMBS|55=USDKRW|268=1|269=0|270=1450.00|\
         272=20260202|273=00:00:02.000";
    let (r, _sink) = adapt(no_size);
    assert_eq!(r, Err(AdaptError::NoSize));

    let mut adapter = SnapshotAdapter::new(Venue::Smbs, SCALES, Some(7_000_000));
    let msg = decode(no_size);
    let mut sink = Sink::default();
    assert_eq!(adapter.adapt(&msg, RECV_NS, &mut sink), Ok(1));
    assert_eq!(sink.records[0].quote().unwrap().bid[0].qty, 7_000_000, "the configured default");
}

#[test]
fn a_symbol_too_long_for_the_wire_is_refused() {
    let long = "X".repeat(jeed_wire::SYMBOL_LEN + 1);
    let body = format!(
        "35=W|49=SMBS|56=FRACTAL|34=4|52=20260202-00:00:03.005|\
         262=X|55={long}|268=1|269=1|270=1453.00|271=5000000|\
         272=20260202|273=00:00:03.000"
    );
    let (r, _sink) = adapt(&body);
    assert_eq!(r, Err(AdaptError::SymbolTooLong));
}

#[test]
fn the_session_scale_reads_the_price_it_is_given() {
    // The precision is the session's, from conf, not the message's: the same
    // "1453.00" is a different integer at a different scale.
    let msg = decode(OPEN_SNAPSHOT);
    let _ = msg;
    let raw = wrap(OPEN_SNAPSHOT);
    let f = frame(&raw).unwrap();
    let s0 = parse_md_message(&f, FixScales::new(Scale::S0, Scale::S0)).unwrap();
    assert_eq!(s0.entries()[0].price, 1453, "no decimals");
    let s2 = parse_md_message(&f, FixScales::new(Scale::S2, Scale::S0)).unwrap();
    assert_eq!(s2.entries()[0].price, 145300, "two decimals");
}
