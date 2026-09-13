//! Tests for `src/data/fix/session.rs` — sequence, administration, liveness.

use super::{wrap, DAY_NS, HEARTBEAT, TWO_SIDED_SNAPSHOT};
use jeed_fix::{
    frame, msg_type, parse_admin_message, FixError, MsgType, SeqVerdict, SessionState,
};

fn admin(body: &str) -> jeed_fix::AdminMessage {
    let raw = wrap(body);
    let f = frame(&raw).unwrap();
    parse_admin_message(&f).unwrap()
}

#[test]
fn msg_type_is_read_without_decoding_the_message() {
    let raw = wrap(TWO_SIDED_SNAPSHOT);
    assert_eq!(msg_type(&frame(&raw).unwrap()), Ok(MsgType::MarketDataSnapshot));
    let raw = wrap(HEARTBEAT);
    assert_eq!(msg_type(&frame(&raw).unwrap()), Ok(MsgType::Heartbeat));
    // A body without 35= at all.
    let raw = wrap("34=1|52=20260202-00:00:00.005");
    assert_eq!(msg_type(&frame(&raw).unwrap()), Err(FixError::MissingTag { tag: 35 }));
}

#[test]
fn a_heartbeat_decodes_to_its_sequence_and_clock() {
    let msg = admin(HEARTBEAT);
    assert_eq!(msg.msg_type, MsgType::Heartbeat);
    assert_eq!(msg.msg_seq_num, 70);
    assert_eq!(msg.sending_time, DAY_NS + 30_005_000_000);
    assert!(msg.test_req_id.is_empty());
    assert_eq!(msg.heartbeat_secs, None);
}

#[test]
fn logon_resend_and_reset_fields_decode() {
    let logon = admin("35=A|49=SMBS|56=FRACTAL|34=1|52=20260202-00:00:00.000|108=30|98=0");
    assert_eq!(logon.msg_type, MsgType::Logon);
    assert_eq!(logon.heartbeat_secs, Some(30));

    let resend = admin("35=2|34=5|52=20260202-00:00:00.000|7=10|16=0");
    assert_eq!(resend.msg_type, MsgType::ResendRequest);
    assert_eq!(resend.begin_seq_no, Some(10));
    assert_eq!(resend.end_seq_no, Some(0), "0 means 'through the end'");

    let reset = admin("35=4|34=9|52=20260202-00:00:00.000|36=100|123=Y");
    assert_eq!(reset.msg_type, MsgType::SequenceReset);
    assert_eq!(reset.new_seq_no, Some(100));
    assert!(reset.gap_fill);

    let test = admin("35=1|34=7|52=20260202-00:00:00.000|112=ARE-YOU-THERE");
    assert_eq!(test.msg_type, MsgType::TestRequest);
    assert_eq!(test.test_req_id.as_bytes(), b"ARE-YOU-THERE");

    let dup = admin("35=0|34=7|43=Y|52=20260202-00:00:00.000");
    assert!(dup.poss_dup);
}

#[test]
fn the_admin_decoder_refuses_market_data() {
    let raw = wrap(TWO_SIDED_SNAPSHOT);
    assert_eq!(
        parse_admin_message(&frame(&raw).unwrap()),
        Err(FixError::UnexpectedMsgType { found: b'W' })
    );
}

#[test]
fn in_order_sequence_numbers_advance_one_at_a_time() {
    let mut s = SessionState::new();
    assert_eq!(s.next_expected, 1);
    for seq in 1..=5 {
        assert_eq!(s.observe(seq, false, 1_000 + seq), SeqVerdict::InOrder);
    }
    assert_eq!(s.next_expected, 6);
    assert_eq!(s.observed, 5);
    assert_eq!((s.gaps, s.lost, s.duplicates, s.regressions), (0, 0, 0, 0));
    assert_eq!(s.last_recv_ns, 1_005);
}

#[test]
fn a_gap_is_reported_once_counted_and_stepped_over() {
    // TCP does not lose messages, so a hole means a session incident — but a
    // snapshot feed must not stall behind a resend that may never come.
    let mut s = SessionState::new();
    assert!(s.observe(1, false, 10).is_in_order());
    let verdict = s.observe(5, false, 20);
    assert_eq!(verdict, SeqVerdict::Gap { expected: 2, received: 5, missing: 3 });
    assert_eq!(verdict.resend_range(), Some((2, 4)), "ask back exactly the hole");
    assert!(!verdict.is_in_order());
    assert_eq!((s.gaps, s.lost), (1, 3));
    assert_eq!(s.next_expected, 6);
    assert!(s.observe(6, false, 30).is_in_order(), "the stream continues from the hole");
    assert_eq!(s.gaps, 1, "one hole is reported once");
}

#[test]
fn a_resend_and_a_regression_are_not_the_same_event() {
    let mut s = SessionState::new();
    for seq in 1..=3 {
        s.observe(seq, false, 10);
    }
    // A legitimate retransmission carries PossDupFlag: drop it, stay put.
    assert_eq!(
        s.observe(2, true, 20),
        SeqVerdict::Duplicate { expected: 4, received: 2 }
    );
    assert_eq!(s.next_expected, 4, "a duplicate must not rewind the session");
    assert_eq!((s.duplicates, s.regressions), (1, 0));

    // The same number without the flag is a protocol violation.
    assert_eq!(
        s.observe(2, false, 30),
        SeqVerdict::Regression { expected: 4, received: 2 }
    );
    assert_eq!(s.next_expected, 4);
    assert_eq!((s.duplicates, s.regressions), (1, 1));
    assert_eq!(SeqVerdict::Duplicate { expected: 4, received: 2 }.resend_range(), None);
}

#[test]
fn logon_sets_the_heartbeat_interval_and_logout_clears_the_session() {
    let mut s = SessionState::new();
    assert!(!s.logged_on);
    assert_eq!(s.heartbeat_interval_ns, 0);

    let logon = admin("35=A|34=1|52=20260202-00:00:00.000|108=30");
    s.observe(logon.msg_seq_num, false, 1_000);
    s.apply_admin(&logon);
    assert!(s.logged_on);
    assert_eq!(s.heartbeat_interval_ns, 30_000_000_000);

    let logout = admin("35=5|34=2|52=20260202-00:00:00.000");
    s.observe(logout.msg_seq_num, false, 2_000);
    s.apply_admin(&logout);
    assert!(!s.logged_on);
    assert_eq!(s.heartbeat_interval_ns, 30_000_000_000, "the interval is not unlearned");
}

#[test]
fn a_sequence_reset_moves_the_expectation() {
    let mut s = SessionState::new();
    s.observe(1, false, 10);
    let reset = admin("35=4|34=2|52=20260202-00:00:00.000|36=100");
    s.observe(reset.msg_seq_num, false, 20);
    s.apply_admin(&reset);
    assert_eq!(s.next_expected, 100);
    assert!(s.observe(100, false, 30).is_in_order());

    s.reset_to(1);
    assert_eq!(s.next_expected, 1);
}

#[test]
fn silence_escalates_in_heartbeat_intervals() {
    let mut s = SessionState::new();
    let now = 1_000_000_000_000u64;
    assert!(!s.is_silent(now, 1), "no interval negotiated yet ⇒ never silent");

    s.heartbeat_interval_ns = 30_000_000_000;
    assert!(!s.is_silent(now, 1), "no message seen yet ⇒ never silent");

    s.observe(1, false, now);
    assert_eq!(s.silence_ns(now), 0);
    assert!(!s.is_silent(now + 29_000_000_000, 1));
    assert!(s.is_silent(now + 31_000_000_000, 1), "one interval ⇒ send a TestRequest");
    assert!(!s.is_silent(now + 31_000_000_000, 2));
    assert!(s.is_silent(now + 61_000_000_000, 2), "two intervals ⇒ the session is dead");

    // Neither clock is monotonic: a backwards `now` must not wrap.
    assert_eq!(s.silence_ns(now - 5), 0);
    assert!(!s.is_silent(now - 5, 1));
}
