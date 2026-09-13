//! `jeed_krx::recv::pipeline` — the receive loop with the sockets taken out.

use crate::builders::{A3, B6, M4, Q2, RECV_NS, VENUE_NS, kospi200_book};
use crate::sink::Collect;
use jeed_krx::TrCode;
use jeed_krx::recv::{IsinFilter, Outcome, Pipeline, TrCodeFilter};
use jeed_wire::{WireKind, header_flags};

fn code(s: &str) -> TrCode {
    TrCode::from_message(s.as_bytes()).unwrap()
}

/// The hot feed's shape: a 선물 book, its prints, its bands, and the schedule
/// channel that shares nothing with them.
fn hot() -> TrCodeFilter {
    TrCodeFilter::new(["B601F", "A301F", "Q201F", "M401F", "A701F"].map(code))
}

fn pipeline(isins: IsinFilter, stale_ns: u64) -> Pipeline<Collect> {
    Pipeline::new(hot(), isins, stale_ns, Collect::new())
}

// ── what gets through ───────────────────────────────────────────────────────

#[test]
fn a_wanted_message_is_decoded_and_published() {
    let mut p = pipeline(IsinFilter::all(), 0);
    let out = p.ingest(&B6::kospi200(kospi200_book()).build(), RECV_NS);

    assert!(out.published(), "{out:?}");
    assert_eq!(p.stats().published, 1);
    assert_eq!(p.stats().received, 1);

    let rec = p.sink().last();
    assert_eq!(rec.kind().unwrap(), WireKind::Quote);
    assert_eq!(rec.header.symbol_bytes(), b"KR4101V90009");
    assert_eq!(rec.header.recv_ns, RECV_NS);
}

#[test]
fn the_published_index_names_the_trcode_that_arrived() {
    // This index is the only evidence a socket ever carried a configured code,
    // and therefore the only check on the socket ↔ trcode wiring there is.
    let mut p = pipeline(IsinFilter::all(), 0);
    let Outcome::Published { trcode } = p.ingest(&A3::kospi200().build(), RECV_NS) else {
        panic!("A301F should publish");
    };
    assert_eq!(p.trcodes().code(trcode), Some(code("A301F")));
}

// ── what does not ───────────────────────────────────────────────────────────

#[test]
fn an_unwanted_trcode_is_dropped_and_is_not_reported_as_a_loss() {
    // 우선호가 and 체결 arrive on the same port as everything else the product
    // group publishes. Counting the configured-away majority as drops would
    // bury a real decode failure under it.
    let mut p = Pipeline::new(
        TrCodeFilter::new([code("B601F")]),
        IsinFilter::all(),
        0,
        Collect::new(),
    );
    let out = p.ingest(&A3::kospi200().build(), RECV_NS);

    assert_eq!(out, Outcome::FilteredTrCode);
    assert_eq!(p.stats().filtered_trcode, 1);
    assert_eq!(p.stats().dropped(), 0, "filtered is not dropped");
    assert_eq!(p.sink().drops, 0, "and nothing is reported outward");
    assert_eq!(p.sink().len(), 0);
}

#[test]
fn a_trcode_this_build_does_not_decode_is_counted_apart_from_a_decode_failure() {
    // "No decoder for this code" is a configuration question — why is that
    // channel joined? — and not the same event as a message that arrived
    // malformed.
    let mut p = pipeline(IsinFilter::all(), 0);
    let mut msg = A3::kospi200().build();
    msg[..5].copy_from_slice(b"A701F");

    assert_eq!(p.ingest(&msg, RECV_NS), Outcome::UnknownTrCode(code("A701F")));
    assert_eq!(p.stats().unknown_trcode, 1);
    assert_eq!(p.stats().decode_failed, 0);
    assert_eq!(p.sink().drops, 1, "this one we wanted");
}

#[test]
fn a_message_of_the_wrong_length_never_reaches_a_slot() {
    let mut p = pipeline(IsinFilter::all(), 0);
    let full = B6::kospi200(kospi200_book()).build();

    let out = p.ingest(&full[..full.len() - 1], RECV_NS);
    assert_eq!(out, Outcome::WrongLength { expected: full.len(), actual: full.len() - 1 });
    assert_eq!(p.stats().wrong_length, 1);
    assert_eq!(p.stats().decode_failed, 0, "rejected before the decoder, and before the slot");
    assert_eq!(p.sink().len(), 0);
}

#[test]
fn a_datagram_shorter_than_a_trcode_is_not_a_krx_message() {
    let mut p = pipeline(IsinFilter::all(), 0);
    assert_eq!(p.ingest(b"B60", RECV_NS), Outcome::TooShort);
    assert_eq!(p.stats().too_short, 1);
    assert_eq!(p.sink().drops, 1);
}

#[test]
fn a_decode_failure_publishes_nothing() {
    let mut p = pipeline(IsinFilter::all(), 0);
    let mut msg = B6::kospi200(kospi200_book()).build();
    // A digit where the 매도호가 잔량 belongs, turned into a letter: the length
    // and the end keyword still check out, so only the field reader catches it.
    msg[47 + 18] = b'X';

    assert!(matches!(p.ingest(&msg, RECV_NS), Outcome::Failed(_)));
    assert_eq!(p.stats().decode_failed, 1);
    assert_eq!(p.sink().len(), 0, "never publish a half-filled record");
    assert_eq!(p.sink().drops, 1);
}

#[test]
fn a_failed_decode_does_not_leak_into_the_next_record() {
    // The sink reuses its buffer exactly as a ring slot does, so a decoder that
    // wrote half a record before failing leaves that half behind. The next
    // message has to overwrite every field it owns.
    let mut p = pipeline(IsinFilter::all(), 0);
    let mut bad = B6::kospi200(kospi200_book()).build();
    bad[47 + 18] = b'X';
    assert!(matches!(p.ingest(&bad, RECV_NS), Outcome::Failed(_)));

    assert!(p.ingest(&A3::kospi200().build(), RECV_NS).published());
    let rec = p.sink().last();
    assert_eq!(rec.kind().unwrap(), WireKind::Trade);
    assert_eq!(rec.header.depth, 0, "a Trade has no book, whatever the wreck before it held");
}

// ── the instrument filter ───────────────────────────────────────────────────

#[test]
fn an_unlisted_instrument_is_dropped_before_it_costs_a_decode() {
    let mut p = pipeline(IsinFilter::new([*b"KR4201V90007"]), 0);
    let out = p.ingest(&B6::kospi200(kospi200_book()).build(), RECV_NS);

    assert_eq!(out, Outcome::FilteredIsin);
    assert_eq!(p.stats().filtered_isin, 1);
    assert_eq!(p.stats().decode_failed, 0);
    assert_eq!(p.sink().len(), 0);
}

#[test]
fn the_instrument_filter_reads_each_header_shape_where_it_actually_sits() {
    // 종목코드 is at 17 on a shape-A 전문 and at 15 on `V1`/`Q2`, which carry no
    // 세션ID. One offset applied to both would compare six bytes of 종목코드
    // against six bytes of something else and pass or fail at random.
    let mut p = pipeline(IsinFilter::new([*b"KR4101V90009"]), 0);

    assert!(p.ingest(&B6::kospi200(kospi200_book()).build(), RECV_NS).published());
    assert!(p.ingest(&Q2::kospi200().build(), RECV_NS).published());
    assert_eq!(p.stats().filtered_isin, 0);
}

#[test]
fn the_schedule_channel_is_not_instrument_scoped() {
    // A session-start notice names no instrument — its 종목코드 is twelve
    // spaces — and a market-wide halt is exactly the message an instrument
    // filter must not be able to drop.
    let mut p = pipeline(IsinFilter::new([*b"KR4101V90009"]), 0);
    let out = p.ingest(&M4::derivative_session_start().build(), RECV_NS);

    assert!(out.published(), "{out:?}");
    assert_eq!(p.sink().last().kind().unwrap(), WireKind::MarketSchedule);
}

// ── stale ───────────────────────────────────────────────────────────────────

#[test]
fn an_old_exchange_timestamp_earns_the_stale_flag() {
    // The builders' message is stamped 76.5 ms before it was received.
    let mut p = pipeline(IsinFilter::all(), 50_000_000);
    assert!(p.ingest(&A3::kospi200().build(), RECV_NS).published());

    let rec = p.sink().last();
    assert_eq!(rec.header.venue_ns, VENUE_NS);
    assert_ne!(rec.header.flags & header_flags::STALE, 0);
    assert_eq!(p.stats().stale, 1);
}

#[test]
fn a_fresh_one_does_not() {
    let mut p = pipeline(IsinFilter::all(), 500_000_000);
    assert!(p.ingest(&A3::kospi200().build(), RECV_NS).published());

    assert_eq!(p.sink().last().header.flags & header_flags::STALE, 0);
    assert_eq!(p.stats().stale, 0);
}

#[test]
fn a_zero_threshold_turns_the_check_off() {
    // Conf has to write the zero on purpose. Reaching it by omission is what
    // once left a book frozen for twenty minutes.
    let mut p = pipeline(IsinFilter::all(), 0);
    assert!(p.ingest(&A3::kospi200().build(), RECV_NS).published());
    assert_eq!(p.stats().stale, 0);
}

#[test]
fn a_venue_clock_ahead_of_ours_is_skew_not_age() {
    // `UnixNano` is unsigned, so a plain subtraction here would wrap and read
    // as an age of about six hundred years.
    let mut p = pipeline(IsinFilter::all(), 1);
    assert!(p.ingest(&A3::kospi200().build(), VENUE_NS - 1_000_000).published());
    assert_eq!(p.stats().stale, 0, "the record is from the future, not from last century");
}

#[test]
fn a_record_with_no_exchange_timestamp_is_never_stale() {
    // `M4` carries a board event time, not a 매매처리시각; judging its age
    // against a field it does not have would flag every schedule notice.
    let mut p = pipeline(IsinFilter::all(), 1);
    assert!(p.ingest(&M4::derivative_session_start().build(), RECV_NS).published());
    assert_eq!(p.stats().stale, 0);
}

// ── heartbeat ───────────────────────────────────────────────────────────────

#[test]
fn a_heartbeat_carries_the_counters_and_is_not_one_of_them() {
    let mut p = pipeline(IsinFilter::all(), 0);
    p.ingest(&A3::kospi200().build(), RECV_NS);
    p.ingest(&B6::kospi200(kospi200_book()).build(), RECV_NS);
    p.heartbeat(RECV_NS + 100_000_000);

    let rec = p.sink().last();
    assert_eq!(rec.kind().unwrap(), WireKind::Heartbeat);
    let hb = rec.heartbeat().unwrap();
    assert_eq!(hb.received, 2);
    assert_eq!(hb.forwarded, 2);

    assert_eq!(p.stats().heartbeats, 1);
    assert_eq!(p.stats().published, 2, "a heartbeat is not market data");
}

#[test]
fn a_heartbeat_from_a_feed_that_has_received_nothing_still_says_it_is_alive() {
    // The whole point: a quiet market and a dead producer look identical on the
    // ring until one of them keeps writing.
    let mut p = pipeline(IsinFilter::all(), 0);
    p.heartbeat(RECV_NS);

    let rec = p.sink().last();
    assert_eq!(rec.kind().unwrap(), WireKind::Heartbeat);
    assert_eq!(rec.header.recv_ns, RECV_NS);
    assert_eq!(rec.heartbeat().unwrap().received, 0);
}
