//! Tests for `src/recv/ws/frame.rs`.

use jeed_crypto::recv::ws::{
    FrameHeader, MAX_CONTROL_PAYLOAD, Opcode, WsError, close_code, framed_len, parse_header,
    write_close, write_frame,
};

fn header(buf: &[u8]) -> FrameHeader {
    parse_header(buf).expect("valid").expect("complete")
}

#[test]
fn a_short_text_frame() {
    let h = header(&[0x81, 0x05, b'h', b'e', b'l', b'l', b'o']);
    assert_eq!(h, FrameHeader { fin: true, opcode: Opcode::Text, header_len: 2, payload_len: 5 });
    assert_eq!(h.len(), 7);
}

#[test]
fn a_16_bit_length() {
    let h = header(&[0x82, 126, 0x01, 0x00]);
    assert_eq!(h.opcode, Opcode::Binary);
    assert_eq!(h.header_len, 4);
    assert_eq!(h.payload_len, 256);
}

#[test]
fn a_64_bit_length() {
    let h = header(&[0x81, 127, 0, 0, 0, 0, 0, 0x01, 0x00, 0x00]);
    assert_eq!(h.header_len, 10);
    assert_eq!(h.payload_len, 65_536);
}

#[test]
fn a_64_bit_length_with_the_high_bit_set_is_refused() {
    assert_eq!(parse_header(&[0x81, 127, 0x80, 0, 0, 0, 0, 0, 0, 0]), Err(WsError::Length));
}

#[test]
fn an_incomplete_header_asks_for_more() {
    assert_eq!(parse_header(&[]), Ok(None));
    assert_eq!(parse_header(&[0x81]), Ok(None));
    assert_eq!(parse_header(&[0x81, 126, 0x01]), Ok(None));
    assert_eq!(parse_header(&[0x81, 127, 0, 0, 0, 0, 0, 0, 1]), Ok(None));
}

#[test]
fn a_continuation_without_fin() {
    let h = header(&[0x00, 0x03, 1, 2, 3]);
    assert!(!h.fin);
    assert_eq!(h.opcode, Opcode::Continuation);
}

#[test]
fn the_server_must_not_mask() {
    assert_eq!(parse_header(&[0x81, 0x85, 1, 2, 3, 4, 0]), Err(WsError::MaskedByServer));
}

#[test]
fn reserved_bits_mean_an_extension_we_never_negotiated() {
    // RSV1 is what permessage-deflate sets on a compressed message.
    assert_eq!(parse_header(&[0xC1, 0x02, 0, 0]), Err(WsError::ReservedBits));
}

#[test]
fn unknown_opcodes_are_refused() {
    for op in [0x3, 0x7, 0xB, 0xF] {
        assert_eq!(parse_header(&[0x80 | op, 0x00]), Err(WsError::Opcode(op)), "{op:#x}");
    }
}

#[test]
fn control_frames_may_not_fragment_or_exceed_125_bytes() {
    assert_eq!(parse_header(&[0x09, 0x00]), Err(WsError::FragmentedControl), "ping without FIN");
    assert_eq!(parse_header(&[0x88, 126, 0x00, 0x7E]), Err(WsError::ControlTooLong(126)));
    assert!(parse_header(&[0x89, 125]).is_ok(), "125 is the limit, not past it");
}

#[test]
fn opcodes_round_trip_and_know_which_are_control() {
    for op in [Opcode::Continuation, Opcode::Text, Opcode::Binary, Opcode::Close, Opcode::Ping, Opcode::Pong] {
        assert_eq!(Opcode::from_u8(op.as_u8()), Some(op));
    }
    assert!(Opcode::Close.is_control() && Opcode::Ping.is_control() && Opcode::Pong.is_control());
    assert!(!Opcode::Text.is_control() && !Opcode::Continuation.is_control());
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Undoes the mask a client frame carries, so a test can read what was sent.
fn unmask(frame: &[u8]) -> (Opcode, Vec<u8>) {
    let opcode = Opcode::from_u8(frame[0] & 0x0F).unwrap();
    assert!(frame[1] & 0x80 != 0, "a client frame is masked");
    let (header_len, len) = match frame[1] & 0x7F {
        126 => (4, u16::from_be_bytes([frame[2], frame[3]]) as usize),
        127 => (10, u64::from_be_bytes(frame[2..10].try_into().unwrap()) as usize),
        n => (2, n as usize),
    };
    let mask = &frame[header_len..header_len + 4];
    let payload = frame[header_len + 4..].iter().enumerate().map(|(i, b)| b ^ mask[i & 3]).collect::<Vec<_>>();
    assert_eq!(payload.len(), len);
    (opcode, payload)
}

#[test]
fn a_written_text_frame_is_masked_and_reads_back() {
    let mut out = [0u8; 64];
    let n = write_frame(&mut out, Opcode::Text, b"hello", [1, 2, 3, 4]).unwrap();

    assert_eq!(n, framed_len(5));
    assert_eq!(out[0], 0x81, "FIN + text");
    assert_eq!(out[1], 0x85, "MASK + 5");
    assert_eq!(&out[2..6], &[1, 2, 3, 4]);
    assert_eq!(&out[6..11], &[b'h' ^ 1, b'e' ^ 2, b'l' ^ 3, b'l' ^ 4, b'o' ^ 1]);
    assert_eq!(unmask(&out[..n]), (Opcode::Text, b"hello".to_vec()));
}

#[test]
fn lengths_pick_the_right_encoding() {
    assert_eq!(framed_len(0), 6);
    assert_eq!(framed_len(125), 2 + 4 + 125);
    assert_eq!(framed_len(126), 4 + 4 + 126);
    assert_eq!(framed_len(65_535), 4 + 4 + 65_535);
    assert_eq!(framed_len(65_536), 10 + 4 + 65_536);

    let payload = vec![b'x'; 300];
    let mut out = vec![0u8; 400];
    let n = write_frame(&mut out, Opcode::Text, &payload, [9, 9, 9, 9]).unwrap();
    assert_eq!(out[1], 0x80 | 126);
    assert_eq!(&out[2..4], &300u16.to_be_bytes());
    assert_eq!(unmask(&out[..n]).1, payload);

    let payload = vec![b'y'; 70_000];
    let mut out = vec![0u8; 70_100];
    let n = write_frame(&mut out, Opcode::Text, &payload, [9, 9, 9, 9]).unwrap();
    assert_eq!(out[1], 0x80 | 127);
    assert_eq!(&out[2..10], &70_000u64.to_be_bytes());
    assert_eq!(unmask(&out[..n]).1, payload);
}

#[test]
fn a_frame_that_does_not_fit_is_refused_not_cut() {
    let mut out = [0u8; 10];
    assert_eq!(write_frame(&mut out, Opcode::Text, b"hello", [0; 4]), Err(WsError::NoRoom));
    assert_eq!(out, [0; 10], "nothing was written");
}

#[test]
fn an_oversized_control_frame_is_refused_on_the_way_out_too() {
    let mut out = [0u8; 256];
    let payload = [0u8; MAX_CONTROL_PAYLOAD + 1];
    assert_eq!(
        write_frame(&mut out, Opcode::Ping, &payload, [0; 4]),
        Err(WsError::ControlTooLong(MAX_CONTROL_PAYLOAD + 1))
    );
}

#[test]
fn a_close_frame_carries_its_code_big_endian() {
    let mut out = [0u8; 16];
    let n = write_close(&mut out, 1000, [0, 0, 0, 0]).unwrap();
    let (op, payload) = unmask(&out[..n]);

    assert_eq!(op, Opcode::Close);
    assert_eq!(payload, [0x03, 0xE8]);
    assert_eq!(close_code(&payload), Some(1000));
}

#[test]
fn a_close_code_needs_two_bytes() {
    assert_eq!(close_code(&[]), None);
    assert_eq!(close_code(&[0x03]), None);
    assert_eq!(close_code(&[0x03, 0xE9, b'b', b'y', b'e']), Some(1001));
}
