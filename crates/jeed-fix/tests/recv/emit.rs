//! Tests for `src/recv/emit.rs` — the outbound half.
//!
//! Every message is checked by **decoding it with this crate's own decoder**,
//! which is the one place that is legitimate: the encoder and the decoder were
//! written against the standard separately, and a message the decoder accepts
//! with the right `BodyLength` and `CheckSum` is a message the venue will too.
//! Where a field's spelling matters rather than its value, the raw bytes are
//! asserted directly.

use jeed_fix::recv::emit::civil_from_days;
use jeed_fix::recv::{Emitter, MAX_EMIT_LEN, SubscriptionRequest, SubscriptionType};
use jeed_fix::tagvalue::days_from_civil;
use jeed_fix::{MdEntryType, MsgType, SOH, frame, msg_type, parse_admin_message};

/// 2026-02-02 00:00:30.005 UTC, the clock of the capture's first heartbeat.
const NOW: u64 = 1_769_990_400_000_000_000 + 30_005_000_000;

fn emitter() -> Emitter {
    Emitter::new(b"JEED", b"SMBS")
}

/// The message as text, `|`-delimited, for asserting on spelling.
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace(SOH as char, "|")
}

#[test]
fn a_logon_frames_decodes_and_says_what_it_should() {
    let mut out = [0u8; MAX_EMIT_LEN];
    let n = emitter().logon(30, true, NOW, &mut out).expect("fits");
    let raw = &out[..n];

    // It frames: BodyLength and CheckSum were computed from what was written.
    let f = frame(raw).expect("our own framing must satisfy our own decoder");
    assert_eq!(f.len(), n, "no bytes past the trailer");
    assert_eq!(f.begin_string, b"FIX.4.4");

    let msg = parse_admin_message(&f).expect("a logon decodes");
    assert_eq!(msg.msg_type, MsgType::Logon);
    assert_eq!(msg.msg_seq_num, 1, "a fresh session starts at 1");
    assert_eq!(msg.heartbeat_secs, Some(30));

    let t = text(raw);
    assert!(t.starts_with("8=FIX.4.4|9="), "8= and 9= lead, in that order");
    assert!(t.contains("|35=A|49=JEED|56=SMBS|34=1|52=20260202-00:00:30.005|"));
    assert!(t.contains("|141=Y|"), "reset both directions");
    assert!(t.ends_with("|"), "the trailer keeps its separator");
}

#[test]
fn the_sequence_number_advances_once_per_message() {
    let mut e = emitter();
    let mut out = [0u8; MAX_EMIT_LEN];
    assert_eq!(e.next_seq(), 1);
    for expected in 1..=4u64 {
        let n = e.heartbeat(None, NOW, &mut out).expect("fits");
        let f = frame(&out[..n]).expect("frames");
        assert_eq!(parse_admin_message(&f).unwrap().msg_seq_num, expected);
    }
    assert_eq!(e.next_seq(), 5);

    // A reconnect starts over — a Logon numbered 4831 against a venue that
    // expects 1 fails on its first message.
    e.reset();
    assert_eq!(e.next_seq(), 1);
    assert_eq!(e.last_sent_ns(), 0);
}

#[test]
fn a_heartbeat_answering_a_test_request_echoes_its_id() {
    let mut out = [0u8; MAX_EMIT_LEN];
    let n = emitter().heartbeat(Some(b"ARE-YOU-THERE"), NOW, &mut out).expect("fits");
    let f = frame(&out[..n]).expect("frames");
    let msg = parse_admin_message(&f).expect("decodes");
    assert_eq!(msg.msg_type, MsgType::Heartbeat);
    assert_eq!(msg.test_req_id.as_bytes(), b"ARE-YOU-THERE");

    // Without one it carries no 112 at all, rather than an empty one.
    let n = emitter().heartbeat(None, NOW, &mut out).expect("fits");
    assert!(!text(&out[..n]).contains("|112="));
}

#[test]
fn the_session_messages_carry_their_own_fields() {
    let mut out = [0u8; MAX_EMIT_LEN];

    let n = emitter().test_request(b"JEED", NOW, &mut out).expect("fits");
    let f = frame(&out[..n]).unwrap();
    let msg = parse_admin_message(&f).unwrap();
    assert_eq!(msg.msg_type, MsgType::TestRequest);
    assert_eq!(msg.test_req_id.as_bytes(), b"JEED");

    let n = emitter().resend_request(2, 4, NOW, &mut out).expect("fits");
    let msg = parse_admin_message(&frame(&out[..n]).unwrap()).unwrap();
    assert_eq!(msg.msg_type, MsgType::ResendRequest);
    assert_eq!((msg.begin_seq_no, msg.end_seq_no), (Some(2), Some(4)));

    let n = emitter().resend_request(2, 0, NOW, &mut out).expect("fits");
    let msg = parse_admin_message(&frame(&out[..n]).unwrap()).unwrap();
    assert_eq!(msg.end_seq_no, Some(0), "0 means 'through the end'");

    let n = emitter().sequence_reset(9, true, NOW, &mut out).expect("fits");
    let msg = parse_admin_message(&frame(&out[..n]).unwrap()).unwrap();
    assert_eq!(msg.msg_type, MsgType::SequenceReset);
    assert_eq!(msg.new_seq_no, Some(9));
    assert!(msg.gap_fill, "we have nothing to resend and say so");

    let n = emitter().logout(Some(b"handler stopping"), NOW, &mut out).expect("fits");
    let msg = parse_admin_message(&frame(&out[..n]).unwrap()).unwrap();
    assert_eq!(msg.msg_type, MsgType::Logout);
    // A logout with no reason is indistinguishable from a crash in their log.
    assert!(text(&out[..n]).contains("|58=handler stopping|"));
}

#[test]
fn a_market_data_request_lists_its_entry_types_before_its_symbols() {
    let mut out = [0u8; MAX_EMIT_LEN];
    let req = SubscriptionRequest {
        req_id: b"USDKRW-SMBS",
        subscription: SubscriptionType::SnapshotPlusUpdates,
        depth: 1,
        update_type: Some(b'1'),
        entry_types: &[MdEntryType::Bid, MdEntryType::Offer, MdEntryType::Trade],
        symbols: &[b"USDKRW", b"EURKRW"],
    };
    let n = emitter().market_data_request(&req, NOW, &mut out).expect("fits");
    let f = frame(&out[..n]).expect("frames");
    assert_eq!(msg_type(&f), Ok(MsgType::MarketDataRequest));

    let t = text(&out[..n]);
    assert!(t.contains("|262=USDKRW-SMBS|263=1|264=1|265=1|"));
    assert!(t.contains("|267=3|269=0|269=1|269=2|"), "the count leads its group");
    assert!(t.contains("|146=2|55=USDKRW|55=EURKRW|"));

    // No update type means the tag is absent, for a venue that only speaks W.
    let req = SubscriptionRequest { update_type: None, ..req };
    let n = emitter().market_data_request(&req, NOW, &mut out).expect("fits");
    assert!(!text(&out[..n]).contains("|265="));
}

#[test]
fn a_message_that_does_not_fit_is_refused_rather_than_truncated() {
    // A truncated FIX message on the wire desynchronises the session at the
    // other end, which is worse than not sending one.
    let mut tiny = [0u8; 16];
    assert!(emitter().logon(30, true, NOW, &mut tiny).is_err());

    let symbols: Vec<&[u8]> = (0..512).map(|_| b"USDKRW".as_slice()).collect();
    let req = SubscriptionRequest {
        req_id: b"R",
        subscription: SubscriptionType::SnapshotPlusUpdates,
        depth: 0,
        update_type: None,
        entry_types: &[MdEntryType::Bid],
        symbols: &symbols,
    };
    let mut out = [0u8; MAX_EMIT_LEN];
    assert!(
        emitter().market_data_request(&req, NOW, &mut out).is_err(),
        "a subscription list past the buffer is an error, not a short list"
    );
}

#[test]
fn the_checksum_is_of_the_bytes_actually_written() {
    let mut out = [0u8; MAX_EMIT_LEN];
    let n = emitter().logon(30, true, NOW, &mut out).expect("fits");

    let sum: u32 = out[..n - 7].iter().map(|b| u32::from(*b)).sum();
    let carried: u32 = String::from_utf8_lossy(&out[n - 4..n - 1]).parse().unwrap();
    assert_eq!(carried, sum % 256);

    // And one flipped byte is caught by the decoder, which is the whole point.
    out[n - 20] ^= 1;
    assert!(frame(&out[..n]).is_err());
}

#[test]
fn the_two_date_tables_are_inverses_of_each_other() {
    // `days_from_civil` is pinned against the capture; this is the other
    // direction, and the encoder's timestamps ride on it.
    for days in [-719_468i64, -1, 0, 11_017, 20_486, 20_500, 100_000] {
        let (y, m, d) = civil_from_days(days);
        assert_eq!(days_from_civil(y, m, d), days, "round trip at {days}");
    }
    assert_eq!(civil_from_days(0), (1970, 1, 1));
    assert_eq!(civil_from_days(-1), (1969, 12, 31));
    // 2026-02-02, the first day of the SMBS capture.
    assert_eq!(civil_from_days(20_486), (2026, 2, 2));
    // A leap day, which is where a hand-rolled table goes wrong.
    assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
}

#[test]
fn sending_time_is_milliseconds_of_our_own_clock() {
    let mut out = [0u8; MAX_EMIT_LEN];
    // Midnight exactly, then one nanosecond short of the next second.
    let n = emitter().heartbeat(None, 1_769_990_400_000_000_000, &mut out).expect("fits");
    assert!(text(&out[..n]).contains("|52=20260202-00:00:00.000|"));

    let n = emitter().heartbeat(None, 1_769_990_400_999_999_999, &mut out).expect("fits");
    assert!(
        text(&out[..n]).contains("|52=20260202-00:00:00.999|"),
        "truncated to ms, never rounded up into the next second"
    );

    let n = emitter().heartbeat(None, 1_769_990_400_000_000_000 + 86_399_000_000_000, &mut out)
        .expect("fits");
    assert!(text(&out[..n]).contains("|52=20260202-23:59:59.000|"));
}

#[test]
fn our_own_silence_is_measured_from_the_last_thing_we_sent() {
    let mut e = emitter();
    let mut out = [0u8; MAX_EMIT_LEN];
    let interval = 30_000_000_000u64;

    assert!(!e.owes_heartbeat(NOW, interval), "nothing sent yet, nothing owed");
    e.logon(30, true, NOW, &mut out).expect("fits");
    assert!(!e.owes_heartbeat(NOW + interval - 1, interval));
    assert!(e.owes_heartbeat(NOW + interval, interval));

    // Zero disables it, and a clock that stepped backwards is not a debt.
    assert!(!e.owes_heartbeat(NOW + interval, 0));
    assert!(!e.owes_heartbeat(NOW - 5, interval));
}

#[test]
fn a_venue_on_another_version_gets_the_begin_string_it_expects() {
    let mut out = [0u8; MAX_EMIT_LEN];
    let n = emitter()
        .with_begin_string(b"FIX.4.2")
        .heartbeat(None, NOW, &mut out)
        .expect("fits");
    let f = frame(&out[..n]).expect("framing does not judge the version");
    assert_eq!(f.begin_string, b"FIX.4.2");
}
