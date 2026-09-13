//! Tests for `src/recv/pipeline.rs` — routing, the sequence conversation, the
//! venue seam, and the age check.
//!
//! Everything here runs without a socket, which is the point of the stage
//! existing (`documents/todo.md` §15).

use super::adapter::{FakeError, FakeVenue, wire_symbol};
use super::builders::{DAY_NS, HEARTBEAT, OPEN_SNAPSHOT, TRADE_INCREMENTAL, TWO_SIDED_SNAPSHOT, wrap};
use super::sink::Collect;
use jeed_fix::recv::{Ingested, Outcome, Pipeline, Reply, SymbolFilter};
use jeed_fix::{MsgType, SeqVerdict, frame};
use jeed_wire::{WireKind, header_flags};

/// A pipeline that keeps everything and flags nothing stale.
fn pipeline() -> Pipeline<FakeVenue, Collect> {
    Pipeline::new(SymbolFilter::all(), 0, FakeVenue::default(), Collect::new())
}

fn feed(
    p: &mut Pipeline<FakeVenue, Collect>,
    body: &str,
    recv_ns: u64,
) -> Ingested<FakeError> {
    let raw = wrap(body);
    let f = frame(&raw).expect("framing must succeed before ingest");
    p.ingest(&f, recv_ns)
}

#[test]
fn a_snapshot_becomes_one_quote_record() {
    let mut p = pipeline();
    let out = feed(&mut p, OPEN_SNAPSHOT, DAY_NS);
    assert_eq!(out.outcome, Outcome::Published { records: 1 });
    assert_eq!(out.verdict, Some(SeqVerdict::InOrder));
    assert_eq!(out.reply, None);
    assert!(!out.disconnect);

    let rec = p.sink().last();
    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.symbol, wire_symbol(b"USDKRW").unwrap());
    assert_eq!(rec.header.venue_time(), Some(DAY_NS));
    // 35=W with only an offer: the missing side is missing, not zero.
    assert_eq!(rec.quote().expect("a quote").ask[0].price, 145_300);
    assert!(rec.header.has(header_flags::BID_EMPTY));
    assert!(!rec.header.has(header_flags::ASK_EMPTY));
    assert_eq!(p.stats().published, 1);
    assert_eq!(p.stats().market_data, 1);
}

#[test]
fn one_incremental_can_publish_several_records() {
    let mut p = pipeline();
    p.session_mut().reset_to(9);
    let out = feed(
        &mut p,
        "35=X|34=9|52=20260202-00:00:14.005|262=R|268=2|\
         279=0|269=2|55=USDKRW|270=1451.00|271=1000000|272=20260202|273=00:00:14.000|\
         279=0|269=2|55=USDKRW|270=1451.10|271=2000000|272=20260202|273=00:00:15.000",
        DAY_NS,
    );
    assert_eq!(out.outcome, Outcome::Published { records: 2 }, "two prints, two records");
    assert_eq!(p.sink().len(), 2);
    assert_eq!(p.sink().records[0].trade().expect("a trade").price, 145_100);
    assert_eq!(p.sink().records[1].trade().expect("a trade").qty, 2_000_000);
    assert_eq!(p.stats().published, 2);
    assert_eq!(p.stats().received, 1, "records are not messages");
}

#[test]
fn an_adapter_refusal_publishes_nothing_and_is_counted_apart() {
    // The venue's own rule: a 35=X that moves the book has nothing to map onto
    // a wire format with no delta record, so it refuses rather than drop.
    let mut p = pipeline();
    p.session_mut().reset_to(4);
    let out = feed(
        &mut p,
        "35=X|34=4|52=20260202-00:00:14.005|268=1|279=1|269=0|55=USDKRW|270=1450.00|271=1000000",
        DAY_NS,
    );
    assert_eq!(out.outcome, Outcome::Refused(FakeError::IncrementalBook));
    assert_eq!(p.sink().len(), 0, "nothing half-mapped reaches the ring");
    assert_eq!(p.sink().drops, 1);
    assert_eq!(p.stats().refused, 1);
    assert_eq!(p.stats().dropped(), 1);
    assert_eq!(p.stats().decode_failed, 0, "a refusal is not a decode failure");
}

#[test]
fn a_decode_failure_publishes_nothing() {
    let mut p = pipeline();
    // 268 says three entries and one is present.
    let out = feed(
        &mut p,
        "35=W|34=1|52=20260202-00:00:14.005|55=USDKRW|268=3|269=0|270=1450.00|271=1",
        DAY_NS,
    );
    assert!(matches!(out.outcome, Outcome::Failed(_)));
    assert_eq!(out.verdict, None, "a message that did not decode has no sequence verdict");
    assert_eq!(p.sink().len(), 0);
    assert_eq!(p.sink().drops, 1);
    assert_eq!(p.stats().decode_failed, 1);
}

#[test]
fn a_gap_is_published_and_asked_back_at_the_same_time() {
    // TCP does not lose messages, so the hole is an incident — but the message
    // that exposed it describes a market that has moved, and holding it back
    // would stall the book behind a resend that may never come.
    let mut p = pipeline();
    feed(&mut p, OPEN_SNAPSHOT, DAY_NS);
    let out = feed(
        &mut p,
        "35=W|34=5|52=20260202-00:00:01.005|55=USDKRW|268=1|269=1|270=1453.00|271=5000000",
        DAY_NS,
    );
    assert_eq!(out.outcome, Outcome::Published { records: 1 }, "published anyway");
    assert_eq!(out.reply, Some(Reply::ResendRequest { begin: 2, end: 4 }), "ask back the hole");
    assert_eq!(out.verdict, Some(SeqVerdict::Gap { expected: 2, received: 5, missing: 3 }));
    assert_eq!((p.stats().gaps, p.stats().lost), (1, 3));
}

#[test]
fn a_duplicate_changes_nothing_and_a_regression_ends_the_session() {
    let mut p = pipeline();
    feed(&mut p, OPEN_SNAPSHOT, DAY_NS);
    feed(&mut p, TWO_SIDED_SNAPSHOT, DAY_NS);
    assert_eq!(p.session().next_expected, 3);

    // A legitimate retransmission carries PossDupFlag.
    let out = feed(
        &mut p,
        "35=W|34=2|43=Y|52=20260202-00:00:01.005|55=USDKRW|268=0",
        DAY_NS,
    );
    assert_eq!(out.outcome, Outcome::Duplicate);
    assert_eq!(out.reply, None, "a resend we already have is not worth answering");
    assert_eq!(p.session().next_expected, 3, "a duplicate must not rewind the session");
    assert_eq!(p.sink().len(), 2);

    // The same number without the flag is a protocol violation.
    let out = feed(&mut p, "35=W|34=2|52=20260202-00:00:01.005|55=USDKRW|268=0", DAY_NS);
    assert_eq!(out.outcome, Outcome::Regression);
    assert!(matches!(out.reply, Some(Reply::Logout { .. })), "the spec answers with a logout");
    assert!(out.disconnect);
    assert_eq!((p.stats().duplicates, p.stats().regressions), (1, 1));
}

#[test]
fn a_test_request_is_answered_with_its_own_id() {
    let mut p = pipeline();
    let out = feed(&mut p, "35=1|34=1|52=20260202-00:00:00.000|112=ARE-YOU-THERE", DAY_NS);
    assert_eq!(out.outcome, Outcome::Admin(MsgType::TestRequest));
    match out.reply {
        Some(Reply::Heartbeat { test_req_id }) => {
            assert_eq!(test_req_id.as_bytes(), b"ARE-YOU-THERE");
        }
        other => panic!("expected a heartbeat, got {other:?}"),
    }
    assert!(!out.disconnect);
}

#[test]
fn a_resend_request_gets_a_gap_fill_because_we_have_nothing_to_resend() {
    // Everything this handler sends is session administration. Replaying a
    // heartbeat from ten minutes ago is worse than saying so.
    let mut p = pipeline();
    let out = feed(&mut p, "35=2|34=1|52=20260202-00:00:00.000|7=10|16=0", DAY_NS);
    assert_eq!(out.outcome, Outcome::Admin(MsgType::ResendRequest));
    assert_eq!(out.reply, Some(Reply::SequenceReset { new_seq: 0 }));
}

#[test]
fn a_logout_is_acknowledged_and_ends_the_connection() {
    let mut p = pipeline();
    let out = feed(&mut p, "35=5|34=1|52=20260202-00:00:00.000|58=end of day", DAY_NS);
    assert_eq!(out.outcome, Outcome::Admin(MsgType::Logout));
    assert!(matches!(out.reply, Some(Reply::Logout { .. })));
    assert!(out.disconnect);
    assert!(!p.session().logged_on);
}

#[test]
fn a_logon_sets_the_heartbeat_interval_and_is_counted() {
    let mut p = pipeline();
    let out = feed(&mut p, "35=A|34=1|52=20260202-00:00:00.000|108=30|98=0", DAY_NS);
    assert_eq!(out.outcome, Outcome::Admin(MsgType::Logon));
    assert_eq!(out.reply, None, "we started this conversation; the Logon back ends it");
    assert!(p.session().logged_on);
    assert_eq!(p.session().heartbeat_interval_ns, 30_000_000_000);
    assert_eq!(p.stats().logons, 1);
}

#[test]
fn a_sequence_reset_without_gap_fill_is_not_judged_on_its_own_number() {
    // The protocol says to apply it whatever 34 it carries, which is the one
    // message whose number the session deliberately ignores.
    let mut p = pipeline();
    feed(&mut p, OPEN_SNAPSHOT, DAY_NS);
    let out = feed(&mut p, "35=4|34=99|52=20260202-00:00:00.000|36=100", DAY_NS);
    assert_eq!(out.verdict, None);
    assert_eq!(p.session().next_expected, 100);
    assert_eq!(p.stats().gaps, 0, "and it is not a gap either");

    // With GapFillFlag it is a real message and occupies its number.
    let mut p = pipeline();
    feed(&mut p, OPEN_SNAPSHOT, DAY_NS);
    let out = feed(&mut p, "35=4|34=2|52=20260202-00:00:00.000|36=100|123=Y", DAY_NS);
    assert_eq!(out.verdict, Some(SeqVerdict::InOrder));
    assert_eq!(p.session().next_expected, 100);
}

#[test]
fn an_unrouted_message_type_is_noticed_rather_than_ignored() {
    // An ExecutionReport here means the order session's credentials ended up in
    // the feed handler's conf.
    let mut p = pipeline();
    let out = feed(&mut p, "35=8|34=1|52=20260202-00:00:00.000", DAY_NS);
    assert_eq!(out.outcome, Outcome::Unhandled(MsgType::Other { first: b'8' }));
    assert_eq!(out.verdict, None, "its sequence number is not ours to judge");
    assert_eq!(p.stats().unhandled, 1);
    assert_eq!(p.stats().dropped(), 0, "it was never something we asked for");
}

#[test]
fn the_symbol_filter_drops_the_message_but_not_what_it_revealed() {
    let mut p = Pipeline::new(
        SymbolFilter::new([b"EURKRW".as_slice()]),
        0,
        FakeVenue::default(),
        Collect::new(),
    );
    p.session_mut().reset_to(5);
    let out = feed(
        &mut p,
        "35=W|34=7|52=20260202-00:00:01.005|55=USDKRW|268=1|269=1|270=1453.00|271=1",
        DAY_NS,
    );
    assert_eq!(out.outcome, Outcome::FilteredSymbol);
    assert_eq!(p.sink().len(), 0);
    assert_eq!(p.sink().drops, 0, "a filtered message was never ours to drop");
    assert_eq!(p.stats().filtered_symbol, 1);
    assert_eq!(
        out.reply,
        Some(Reply::ResendRequest { begin: 5, end: 6 }),
        "the gap is a session fact, whoever the message was about"
    );
}

#[test]
fn the_age_check_runs_on_every_record_whatever_the_adapter_does() {
    // Stale is handler policy with a conf knob behind it, and the consumer is
    // told only the conclusion. An adapter that could forget to apply it would
    // make one venue quietly publish stale books.
    let late = DAY_NS + 1_000_000_000 + 2_000_000_000;

    let mut on = Pipeline::new(
        SymbolFilter::all(),
        500_000_000,
        FakeVenue::default(),
        Collect::new(),
    );
    on.session_mut().reset_to(2);
    feed(&mut on, TWO_SIDED_SNAPSHOT, late);
    assert!(on.sink().last().header.is_stale());
    assert_eq!(on.stats().stale, 1);

    // Zero disables it, which conf has to say on purpose.
    let mut off = pipeline();
    off.session_mut().reset_to(2);
    feed(&mut off, TWO_SIDED_SNAPSHOT, late);
    assert!(!off.sink().last().header.is_stale());
    assert_eq!(off.stats().stale, 0);

    // And a fresh record is not flagged even with the check on.
    let mut on = Pipeline::new(
        SymbolFilter::all(),
        500_000_000,
        FakeVenue::default(),
        Collect::new(),
    );
    on.session_mut().reset_to(2);
    feed(&mut on, TWO_SIDED_SNAPSHOT, DAY_NS + 1_100_000_000);
    assert!(!on.sink().last().header.is_stale());
}

#[test]
fn a_venue_clock_ahead_of_ours_is_skew_not_an_age_of_eighteen_billion_years() {
    let mut p = Pipeline::new(
        SymbolFilter::all(),
        500_000_000,
        FakeVenue::default(),
        Collect::new(),
    );
    p.session_mut().reset_to(2);
    // recv_ns *before* the venue timestamp: UnixNano is unsigned and would wrap.
    feed(&mut p, TWO_SIDED_SNAPSHOT, DAY_NS);
    assert!(!p.sink().last().header.is_stale());
}

#[test]
fn the_heartbeat_record_carries_the_counters_and_the_venue() {
    let mut p = pipeline();
    feed(&mut p, OPEN_SNAPSHOT, DAY_NS);
    feed(&mut p, HEARTBEAT, DAY_NS);
    p.heartbeat(DAY_NS + 30_000_000_000);

    let rec = p.sink().last();
    assert_eq!(rec.kind(), Ok(WireKind::Heartbeat));
    assert_eq!(rec.header.venue, jeed_wire::Venue::Smbs.as_u8(), "the adapter names the venue");
    let beat = rec.heartbeat().expect("a heartbeat");
    assert_eq!(beat.received, 2, "both messages, market data and session alike");
    assert_eq!(beat.forwarded, 1, "records, not messages");
    assert_eq!(p.stats().heartbeats, 1);
}

#[test]
fn a_reconnect_restarts_the_sequence_but_not_the_ledger() {
    let mut p = pipeline();
    feed(&mut p, OPEN_SNAPSHOT, DAY_NS);
    feed(
        &mut p,
        "35=W|34=5|52=20260202-00:00:01.005|55=USDKRW|268=1|269=1|270=1453.00|271=1",
        DAY_NS,
    );
    assert_eq!((p.stats().gaps, p.stats().lost), (1, 3));

    p.reset_session();
    assert_eq!(p.session().next_expected, 1, "the protocol requires it");
    assert_eq!(
        (p.stats().gaps, p.stats().lost),
        (1, 3),
        "a session that relogs on every gap must not show a clean sheet"
    );
    assert_eq!(feed(&mut p, OPEN_SNAPSHOT, DAY_NS).verdict, Some(SeqVerdict::InOrder));
}

#[test]
fn a_trade_incremental_keys_off_the_symbol_inside_the_group() {
    let mut p = pipeline();
    p.session_mut().reset_to(18);
    let out = feed(&mut p, TRADE_INCREMENTAL, DAY_NS);
    assert_eq!(out.outcome, Outcome::Published { records: 1 });
    let rec = p.sink().last();
    assert_eq!(
        rec.header.symbol,
        wire_symbol(b"USDKRW").unwrap(),
        "35=X carries Symbol in the entry"
    );
    assert_eq!(rec.header.venue_time(), Some(DAY_NS + 14_000_000_000));
}
