//! Tests for `src/data/fix/frame.rs` — `BodyLength` / `CheckSum` framing and
//! the streaming receive buffer.

use super::{soh, wrap, wrap_as, HEARTBEAT, TWO_SIDED_SNAPSHOT};
use jeed_fix::{checksum, frame, FixError, FrameBuffer, FIX_MAX_BODY_LENGTH};

#[test]
fn a_good_message_frames_into_begin_string_and_body() {
    let raw = wrap(HEARTBEAT);
    let f = frame(&raw).expect("valid message");
    assert_eq!(f.len(), raw.len());
    assert_eq!(f.bytes, raw.as_slice());
    assert_eq!(f.begin_string, b"FIX.4.4");
    assert_eq!(
        f.body,
        soh(&format!("{HEARTBEAT}|")).as_slice(),
        "the body excludes 8=, 9= and 10=, and keeps its last field separator"
    );
    assert_eq!(*f.body.last().unwrap(), 0x01, "body ends on a field boundary");

    // The capture's own framing arithmetic: 9= counts the body, 10= sums
    // everything before itself.
    let nine = raw.windows(2).position(|w| w == b"9=").unwrap();
    let declared: usize = std::str::from_utf8(&raw[nine + 2..nine + 2 + 2])
        .unwrap()
        .trim_end_matches(|c: char| !c.is_ascii_digit())
        .parse()
        .unwrap_or(0);
    assert!(declared > 0);
    assert_eq!(checksum(&raw[..raw.len() - 7]), {
        let d = std::str::from_utf8(&raw[raw.len() - 4..raw.len() - 1]).unwrap();
        d.parse::<u16>().unwrap() as u8
    });
}

#[test]
fn a_prefix_is_incomplete_not_broken() {
    let raw = wrap(TWO_SIDED_SNAPSHOT);
    for cut in [0, 1, 2, 5, 9, 12, 20, raw.len() - 1] {
        assert_eq!(
            frame(&raw[..cut]),
            Err(FixError::Incomplete),
            "a {cut}-byte prefix must ask for more bytes, not resynchronise"
        );
    }
    assert!(frame(&raw).is_ok());
}

#[test]
fn framing_errors_are_distinguished() {
    // Not FIX at all.
    assert_eq!(frame(b"GET / HTTP/1.1\r\n"), Err(FixError::BeginString));
    assert_eq!(frame(&soh("9=5|35=0|")), Err(FixError::BeginString));

    // A BeginString that never terminates is garbage, not a short read.
    let long = format!("8={}", "F".repeat(40));
    assert_eq!(frame(long.as_bytes()), Err(FixError::BeginString));

    // BodyLength missing or not a number.
    let raw = wrap(HEARTBEAT);
    let mut no_len = raw.clone();
    no_len[10] = b'7';
    assert!(matches!(frame(&no_len), Err(FixError::BodyLength { .. })));

    // A declared length that lands mid-field.
    let mut short = raw.clone();
    let nine = short.windows(2).position(|w| w == b"9=").unwrap();
    short[nine + 2] = b'4';
    assert!(matches!(frame(&short), Err(FixError::BodyLength { .. })));

    // Declared zero.
    let zero = soh("8=FIX.4.4|9=0|10=000|");
    assert_eq!(frame(&zero), Err(FixError::BodyLength { declared: 0 }));

    // An absurd length is rejected without waiting for that many bytes.
    let huge = soh(&format!("8=FIX.4.4|9={}|", FIX_MAX_BODY_LENGTH + 1));
    assert!(matches!(frame(&huge), Err(FixError::BodyLength { .. })));
}

#[test]
fn a_wrong_checksum_fails_with_both_values() {
    let mut raw = wrap(HEARTBEAT);
    let n = raw.len();
    let expected = checksum(&raw[..n - 7]);
    raw[n - 2] = if raw[n - 2] == b'0' { b'1' } else { b'0' };
    match frame(&raw) {
        Err(FixError::CheckSum { expected: e, found }) => {
            assert_eq!(e, expected);
            assert_ne!(found, expected);
        }
        other => panic!("expected a checksum error, got {other:?}"),
    }
}

#[test]
fn a_broken_trailer_is_not_a_checksum_error() {
    let raw = wrap(HEARTBEAT);
    let n = raw.len();

    let mut bad_tag = raw.clone();
    bad_tag[n - 7] = b'1';
    bad_tag[n - 6] = b'1';
    assert_eq!(frame(&bad_tag), Err(FixError::Trailer));

    let mut bad_soh = raw.clone();
    bad_soh[n - 1] = b'0';
    assert_eq!(frame(&bad_soh), Err(FixError::Trailer));

    let mut bad_digits = raw.clone();
    bad_digits[n - 3] = b'x';
    assert_eq!(frame(&bad_digits), Err(FixError::Trailer));
}

#[test]
fn any_begin_string_frames_version_policy_lives_above() {
    let raw = wrap_as("FIX.4.2", HEARTBEAT);
    let f = frame(&raw).expect("framing does not judge the version");
    assert_eq!(f.begin_string, b"FIX.4.2");
}

#[test]
fn the_buffer_reassembles_messages_split_across_reads() {
    let a = wrap(TWO_SIDED_SNAPSHOT);
    let b = wrap(HEARTBEAT);
    let mut stream = a.clone();
    stream.extend_from_slice(&b);

    let mut buf: FrameBuffer<4096> = FrameBuffer::new();
    let mut framed: Vec<Vec<u8>> = Vec::new();
    // Feed the stream three bytes at a time — the worst case for framing.
    for chunk in stream.chunks(3) {
        assert_eq!(buf.push(chunk), chunk.len());
        while let Some(f) = buf.peek().expect("no framing error") {
            let (len, bytes) = (f.len(), f.bytes.to_vec());
            framed.push(bytes);
            buf.consume(len);
        }
    }
    assert_eq!(framed, vec![a, b]);
    assert!(buf.filled().is_empty(), "nothing left over");
}

#[test]
fn the_buffer_accepts_writes_through_spare_and_commit() {
    let raw = wrap(HEARTBEAT);
    let mut buf: FrameBuffer<512> = FrameBuffer::new();
    let n = raw.len();
    buf.spare()[..n].copy_from_slice(&raw);
    buf.commit(n);
    let f = buf.peek().unwrap().expect("one message");
    assert_eq!(f.len(), n);
    buf.consume(n);
    assert!(buf.peek().unwrap().is_none());
}

#[test]
fn a_message_that_cannot_fit_is_reported_not_awaited_forever() {
    let raw = wrap(TWO_SIDED_SNAPSHOT);
    let mut buf: FrameBuffer<64> = FrameBuffer::new();
    assert!(buf.push(&raw) < raw.len(), "the buffer is deliberately too small");
    assert!(
        matches!(buf.peek(), Err(FixError::BodyLength { declared: 64 })),
        "a full buffer that still does not frame is an oversize body, not Incomplete"
    );
    buf.clear();
    assert!(buf.peek().unwrap().is_none());
}

#[test]
fn garbage_ahead_of_a_message_is_reported_so_the_caller_can_resync() {
    let mut stream = b"\x01\x01garbage".to_vec();
    stream.extend_from_slice(&wrap(HEARTBEAT));
    let mut buf: FrameBuffer<512> = FrameBuffer::new();
    buf.push(&stream);
    assert_eq!(buf.peek(), Err(FixError::BeginString));
    // The session is out of sync; the documented reaction is to drop it.
    buf.clear();
    buf.push(&wrap(HEARTBEAT));
    assert!(buf.peek().unwrap().is_some());
}
