//! Tests for `src/recv/receiver.rs` and `src/recv/link.rs` — the loop,
//! against a real socket.
//!
//! The venue is a `TcpListener` on loopback and the test itself plays it: it
//! answers the handshake, writes the frames a server would, and reads back
//! what the handler sends. That is worth more than a mock, because the thing
//! being tested is exactly the part a mock would stand in for — framing
//! across arbitrary read boundaries, fragments with a ping in the middle, a
//! peer that hangs up.
//!
//! Plain TCP, no TLS: `src/recv/link.rs` does not know which it is on, so
//! the loop is tested here and the TLS path once, for real, in `live.rs`.

use crate::frames::SPOT_TRADE;
use crate::router::{Fake, KEEPALIVE};
use crate::sink::Collect;
use jeed_crypto::recv::ws::accept;
use jeed_crypto::recv::{
    CLOSE_GOING_AWAY, CLOSE_NORMAL, CLOSE_PROTOCOL_ERROR, Config, Endpoint, LinkError, LinkOptions,
    LinkState, Mode, Opcode, Receiver, SendError, Tls, WsError,
};
use jeed_wire::WireKind;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::time::{Duration, Instant};

type Rx = Receiver<Fake, Collect>;

/// The venue's end of the connection.
struct Peer {
    stream: TcpStream,
    buf: Vec<u8>,
}

impl Peer {
    /// Accepts the handler and completes the handshake correctly.
    fn accept(listener: &TcpListener) -> Self {
        Self::accept_with(listener, |key| {
            let mut v = b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ".to_vec();
            v.extend_from_slice(&accept(key));
            v.extend_from_slice(b"\r\n\r\n");
            v
        })
    }

    /// Accepts the handler, reads its request, and answers with whatever
    /// `respond` makes of the key.
    fn accept_with(listener: &TcpListener, respond: impl FnOnce(&[u8; 24]) -> Vec<u8>) -> Self {
        let (stream, _) = listener.accept().expect("accept the handler");
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut peer = Self { stream, buf: Vec::new() };

        let request = peer.read_until(b"\r\n\r\n");
        let text = std::str::from_utf8(&request).expect("ascii request");
        assert!(text.starts_with("GET /feed HTTP/1.1\r\n"), "{text}");
        assert!(text.contains("\r\nUpgrade: websocket\r\n"), "{text}");
        let key = text
            .lines()
            .find_map(|l| l.strip_prefix("Sec-WebSocket-Key: "))
            .expect("a key");
        let key: [u8; 24] = key.as_bytes().try_into().expect("24 bytes of base64");

        peer.stream.write_all(&respond(&key)).unwrap();
        peer
    }

    fn read_until(&mut self, needle: &[u8]) -> Vec<u8> {
        loop {
            if let Some(i) = self.buf.windows(needle.len()).position(|w| w == needle) {
                let end = i + needle.len();
                return self.buf.drain(..end).collect();
            }
            self.fill();
        }
    }

    fn fill(&mut self) {
        let mut chunk = [0u8; 4096];
        let n = self.stream.read(&mut chunk).expect("the handler should still be connected");
        assert!(n > 0, "the handler closed the connection");
        self.buf.extend_from_slice(&chunk[..n]);
    }

    /// Writes one server frame — unmasked, as a server must.
    fn frame(&mut self, fin: bool, opcode: Opcode, payload: &[u8]) {
        self.raw(&server_frame(fin, opcode, payload));
    }

    fn text(&mut self, payload: &[u8]) {
        self.frame(true, Opcode::Text, payload);
    }

    fn raw(&mut self, bytes: &[u8]) {
        self.stream.write_all(bytes).expect("write to the handler");
        self.stream.flush().unwrap();
    }

    /// Reads one masked client frame and unmasks it.
    fn recv(&mut self) -> (Opcode, Vec<u8>) {
        loop {
            if let Some((op, payload, len)) = client_frame(&self.buf) {
                self.buf.drain(..len);
                return (op, payload);
            }
            self.fill();
        }
    }

    fn hang_up(self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

/// Encodes a server frame.
fn server_frame(fin: bool, opcode: Opcode, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![(if fin { 0x80 } else { 0 }) | opcode.as_u8()];
    match payload.len() {
        n if n < 126 => v.push(n as u8),
        n if n <= 0xFFFF => {
            v.push(126);
            v.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            v.push(127);
            v.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    v.extend_from_slice(payload);
    v
}

/// Decodes a masked client frame at the head of `buf`, if one is complete.
fn client_frame(buf: &[u8]) -> Option<(Opcode, Vec<u8>, usize)> {
    if buf.len() < 2 {
        return None;
    }
    let opcode = Opcode::from_u8(buf[0] & 0x0F).expect("known opcode");
    assert!(buf[0] & 0x80 != 0, "the handler never fragments");
    assert!(buf[1] & 0x80 != 0, "a client frame must be masked");
    let (hlen, len) = match buf[1] & 0x7F {
        126 => (4, u16::from_be_bytes([*buf.get(2)?, *buf.get(3)?]) as usize),
        127 => (10, u64::from_be_bytes(buf.get(2..10)?.try_into().unwrap()) as usize),
        n => (2, n as usize),
    };
    let total = hlen + 4 + len;
    if buf.len() < total {
        return None;
    }
    let mask = &buf[hlen..hlen + 4];
    let payload = buf[hlen + 4..total].iter().enumerate().map(|(i, b)| b ^ mask[i & 3]).collect();
    Some((opcode, payload, total))
}

fn config() -> Config {
    Config {
        mode: Mode::Block,
        ping_interval_ns: 0,
        record_heartbeat_ns: 0,
        stale_ns: 0,
        reconnect_ns: 0,
        link: LinkOptions { read_timeout: Duration::from_millis(20), ..Default::default() },
        ..Config::default()
    }
}

fn receiver(cfg: Config, port: u16, router: Fake) -> Rx {
    Receiver::new(cfg, Endpoint::new("127.0.0.1", port, "/feed", false), Tls::new(), router, Collect::new())
}

/// Brings up a listener, and a receiver pointed at it, not yet connected.
fn listening(cfg: Config, router: Fake) -> (Rx, TcpListener) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().unwrap().port();
    (receiver(cfg, port, router), listener)
}

/// Connects the receiver on another thread while this one plays the venue's
/// handshake, then hands both back.
fn connected(cfg: Config, router: Fake) -> (Rx, Peer, TcpListener) {
    let (mut rx, listener) = listening(cfg, router);
    let (rx, peer) = handshake(rx.take(), &listener);
    (rx, peer, listener)
}

/// Runs one `poll_once` — which blocks in the handshake — against a peer
/// that answers it.
fn handshake(mut rx: Rx, listener: &TcpListener) -> (Rx, Peer) {
    std::thread::scope(|s| {
        let h = s.spawn(move || {
            assert!(rx.poll_once().is_none(), "connecting to a listening socket must succeed");
            assert_eq!(rx.state(), LinkState::Open);
            rx
        });
        let peer = Peer::accept(listener);
        (h.join().unwrap(), peer)
    })
}

trait Take {
    fn take(&mut self) -> Rx;
}

impl Take for Rx {
    /// Moves the receiver out so a thread can own it for one round.
    fn take(&mut self) -> Rx {
        let stand_in = receiver(config(), 1, Fake::new());
        std::mem::replace(self, stand_in)
    }
}

/// Runs rounds until `done` or the deadline, so a broken loop fails rather
/// than hangs. Returns the errors handed back on the way.
fn pump_until(rx: &mut Rx, mut done: impl FnMut(&Rx) -> bool) -> Vec<LinkError> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut errors = Vec::new();
    while !done(rx) {
        assert!(Instant::now() < deadline, "the loop never got there; errors so far: {errors:?}");
        if let Some(e) = rx.poll_once() {
            errors.push(e);
        }
    }
    errors
}

// ---------------------------------------------------------------------------
// Connecting
// ---------------------------------------------------------------------------

#[test]
fn a_handshake_opens_the_link_and_counts() {
    let (rx, _peer, _l) = connected(config(), Fake::new());
    assert_eq!(rx.state(), LinkState::Open);
    assert_eq!(rx.stats().opens, 1);
    assert_eq!(rx.stats().connect_failures, 0);
}

#[test]
fn nobody_listening_is_a_connect_failure_and_a_reconnect() {
    let mut rx = receiver(config(), 1, Fake::new());
    let e = rx.poll_once().expect("port 1 refuses");
    assert!(matches!(e, LinkError::Io { call: "connect", .. }), "{e}");
    assert_eq!(rx.state(), LinkState::Disconnected);
    assert_eq!(rx.stats().connect_failures, 1);

    // `reconnect_ns` is zero in the test config, so the next round tries again.
    assert!(rx.poll_once().is_some());
    assert_eq!(rx.stats().connect_failures, 2);
}

#[test]
fn a_wrong_accept_fails_the_connection() {
    let (mut rx, listener) = listening(config(), Fake::new());
    let (rx, _peer) = std::thread::scope(|s| {
        let h = s.spawn(move || {
            let e = rx.poll_once().expect("must fail");
            assert_eq!(e.to_string(), WsError::Accept.to_string());
            rx
        });
        let peer = Peer::accept_with(&listener, |_| {
            b"HTTP/1.1 101 OK\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: AAAAAAAAAAAAAAAAAAAAAAAAAAA=\r\n\r\n".to_vec()
        });
        (h.join().unwrap(), peer)
    });
    assert_eq!(rx.state(), LinkState::Disconnected);
    assert_eq!(rx.stats().connect_failures, 1);
    assert_eq!(rx.stats().opens, 0);
}

#[test]
fn a_404_fails_the_connection_with_its_code() {
    let (mut rx, listener) = listening(config(), Fake::new());
    std::thread::scope(|s| {
        let h = s.spawn(move || {
            let e = rx.poll_once().expect("must fail");
            assert!(matches!(e, LinkError::Protocol(WsError::Status(404))), "{e}");
        });
        let _peer = Peer::accept_with(&listener, |_| b"HTTP/1.1 404 Not Found\r\n\r\n".to_vec());
        h.join().unwrap();
    });
}

#[test]
fn a_peer_that_never_answers_the_handshake_times_out() {
    let mut cfg = config();
    cfg.link.handshake_timeout = Duration::from_millis(100);
    let (mut rx, listener) = listening(cfg, Fake::new());
    let (_stream, _) = std::thread::scope(|s| {
        let h = s.spawn(move || {
            let e = rx.poll_once().expect("must fail");
            assert!(matches!(e, LinkError::Timeout("handshake")), "{e}");
        });
        let accepted = listener.accept().unwrap();
        h.join().unwrap();
        accepted
    });
}

// ---------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------

#[test]
fn a_text_frame_becomes_a_record() {
    let (mut rx, mut peer, _l) = connected(config(), Fake::new());
    peer.text(SPOT_TRADE);

    pump_until(&mut rx, |rx| rx.pipeline().sink().len() == 1);
    let rec = rx.pipeline().sink().last();
    assert_eq!(rec.kind(), Ok(WireKind::Trade));
    assert_eq!(rx.stats().frames, 1);
    assert_eq!(rx.stats().messages, 1);
    assert_eq!(rx.stats().published, 1);
    assert!(rx.stats().bytes > SPOT_TRADE.len() as u64);
}

#[test]
fn bytes_after_the_101_are_the_first_frame() {
    // A server that writes its response and its first frame in one segment.
    let (mut rx, listener) = listening(config(), Fake::new());
    let (mut rx, _peer) = std::thread::scope(|s| {
        let h = s.spawn(move || {
            assert!(rx.poll_once().is_none());
            rx
        });
        let peer = Peer::accept_with(&listener, |key| {
            let mut v = b"HTTP/1.1 101 OK\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ".to_vec();
            v.extend_from_slice(&accept(key));
            v.extend_from_slice(b"\r\n\r\n");
            v.extend_from_slice(&server_frame(true, Opcode::Text, SPOT_TRADE));
            v
        });
        (h.join().unwrap(), peer)
    });

    pump_until(&mut rx, |rx| rx.pipeline().sink().len() == 1);
}

#[test]
fn a_frame_split_across_reads_is_framed_once_complete() {
    let (mut rx, mut peer, _l) = connected(config(), Fake::new());
    let frame = server_frame(true, Opcode::Text, SPOT_TRADE);

    peer.raw(&frame[..1]);
    for _ in 0..3 {
        rx.poll_once();
    }
    assert_eq!(rx.stats().frames, 0, "one byte is not a frame");

    peer.raw(&frame[1..40]);
    for _ in 0..3 {
        rx.poll_once();
    }
    assert_eq!(rx.stats().frames, 0, "a header with half a payload is not a frame");

    peer.raw(&frame[40..]);
    pump_until(&mut rx, |rx| rx.stats().frames == 1);
    assert_eq!(rx.pipeline().sink().len(), 1);
}

#[test]
fn several_frames_in_one_read_are_all_framed() {
    let (mut rx, mut peer, _l) = connected(config(), Fake::new());
    let mut burst = Vec::new();
    for _ in 0..5 {
        burst.extend_from_slice(&server_frame(true, Opcode::Text, SPOT_TRADE));
    }
    peer.raw(&burst);

    pump_until(&mut rx, |rx| rx.pipeline().sink().len() == 5);
    assert_eq!(rx.stats().frames, 5);
}

#[test]
fn sixteen_and_sixty_four_bit_lengths_are_framed() {
    let (mut rx, mut peer, _l) = connected(config(), Fake::new());

    let mut medium = br#"{"result":null,"id":1,"pad":""#.to_vec();
    medium.extend(std::iter::repeat_n(b'x', 300));
    medium.extend_from_slice(b"\"}");
    peer.text(&medium);

    let mut large = br#"{"result":null,"id":2,"pad":""#.to_vec();
    large.extend(std::iter::repeat_n(b'y', 70_000));
    large.extend_from_slice(b"\"}");
    peer.text(&large);

    pump_until(&mut rx, |rx| rx.stats().messages == 2);
    assert_eq!(rx.stats().published, 0, "acknowledgements publish nothing");
    assert_eq!(rx.stats().decode_failed, 0);
}

#[test]
fn a_fragmented_message_with_a_ping_in_the_middle() {
    let (mut rx, mut peer, _l) = connected(config(), Fake::new());
    let (a, rest) = SPOT_TRADE.split_at(30);
    let (b, c) = rest.split_at(50);

    peer.frame(false, Opcode::Text, a);
    peer.frame(true, Opcode::Ping, b"mid");
    peer.frame(false, Opcode::Continuation, b);
    peer.frame(true, Opcode::Continuation, c);

    pump_until(&mut rx, |rx| rx.pipeline().sink().len() == 1);
    assert_eq!(rx.pipeline().sink().last().kind(), Ok(WireKind::Trade));
    assert_eq!(rx.stats().frames, 4);
    assert_eq!(rx.stats().fragments, 3);
    assert_eq!(rx.stats().messages, 1);
    assert_eq!(rx.stats().pings, 1);
    assert_eq!(rx.stats().pongs_sent, 1);
    assert_eq!(peer.recv(), (Opcode::Pong, b"mid".to_vec()), "the pong echoes the ping's payload");
}

#[test]
fn a_ping_is_answered_with_its_payload() {
    let (mut rx, mut peer, _l) = connected(config(), Fake::new());
    peer.frame(true, Opcode::Ping, b"");
    peer.frame(true, Opcode::Ping, b"1694000000000");

    pump_until(&mut rx, |rx| rx.stats().pongs_sent == 2);
    assert_eq!(peer.recv(), (Opcode::Pong, Vec::new()));
    assert_eq!(peer.recv(), (Opcode::Pong, b"1694000000000".to_vec()));
    assert_eq!(rx.stats().messages, 0);
}

#[test]
fn an_unsolicited_pong_is_counted_and_ignored() {
    let (mut rx, mut peer, _l) = connected(config(), Fake::new());
    peer.frame(true, Opcode::Pong, b"hi");
    pump_until(&mut rx, |rx| rx.stats().pongs == 1);
    assert_eq!(rx.stats().messages, 0);
}

#[test]
fn a_binary_frame_is_counted_and_not_routed() {
    let (mut rx, mut peer, _l) = connected(config(), Fake::new());
    peer.frame(true, Opcode::Binary, SPOT_TRADE);
    pump_until(&mut rx, |rx| rx.stats().binary == 1);
    assert_eq!(rx.stats().messages, 0);
    assert_eq!(rx.pipeline().sink().len(), 0);
    assert_eq!(rx.state(), LinkState::Open, "not a protocol error");
}

#[test]
fn a_message_the_router_refuses_is_counted_and_the_link_survives() {
    let (mut rx, mut peer, _l) = connected(config(), Fake::new());
    peer.text(br#"{"e":"trade","s":"ETHUSDT"}"#);
    peer.text(SPOT_TRADE);

    pump_until(&mut rx, |rx| rx.pipeline().sink().len() == 1);
    assert_eq!(rx.stats().decode_failed, 1);
    assert_eq!(rx.pipeline().sink().drops, 1);
    assert_eq!(rx.state(), LinkState::Open);
}

// ---------------------------------------------------------------------------
// Protocol errors and closing
// ---------------------------------------------------------------------------

#[test]
fn a_masked_server_frame_tears_the_connection_down() {
    let (mut rx, mut peer, _l) = connected(config(), Fake::new());
    peer.raw(&[0x81, 0x85, 1, 2, 3, 4, 0]);

    let errors = pump_until(&mut rx, |rx| rx.state() == LinkState::Disconnected);
    assert!(matches!(errors[0], LinkError::Protocol(WsError::MaskedByServer)), "{errors:?}");
    assert_eq!(rx.stats().protocol_errors, 1);
    assert_eq!(rx.stats().disconnects, 1);
}

#[test]
fn a_data_frame_inside_a_fragmented_message_is_a_protocol_error() {
    let (mut rx, mut peer, _l) = connected(config(), Fake::new());
    peer.frame(false, Opcode::Text, b"{\"a\":");
    peer.frame(true, Opcode::Text, b"{}");

    let errors = pump_until(&mut rx, |rx| rx.state() == LinkState::Disconnected);
    assert!(matches!(errors[0], LinkError::Protocol(WsError::InterleavedMessage)), "{errors:?}");
    assert_eq!(peer.recv().0, Opcode::Close, "the handler says why it is leaving");
}

#[test]
fn a_protocol_error_is_answered_with_close_1002() {
    let (mut rx, mut peer, _l) = connected(config(), Fake::new());
    peer.frame(true, Opcode::Continuation, b"orphan");

    pump_until(&mut rx, |rx| rx.state() == LinkState::Disconnected);
    let (op, payload) = peer.recv();
    assert_eq!(op, Opcode::Close);
    assert_eq!(payload, CLOSE_PROTOCOL_ERROR.to_be_bytes());
}

#[test]
fn a_close_frame_is_echoed_and_reported_with_its_code() {
    let (mut rx, mut peer, _l) = connected(config(), Fake::new());
    peer.frame(true, Opcode::Close, &[0x03, 0xE9, b'b', b'y', b'e']);

    let errors = pump_until(&mut rx, |rx| rx.state() == LinkState::Disconnected);
    assert!(matches!(errors[0], LinkError::Closed { code: Some(1001) }), "{errors:?}");
    assert!(errors[0].is_closed());
    assert_eq!(rx.stats().closes, 1);
    assert_eq!(rx.stats().disconnects, 1);

    let (op, payload) = peer.recv();
    assert_eq!(op, Opcode::Close);
    assert_eq!(payload, CLOSE_NORMAL.to_be_bytes());
}

#[test]
fn a_peer_that_hangs_up_is_closed_not_an_io_error() {
    let (mut rx, peer, _l) = connected(config(), Fake::new());
    peer.hang_up();

    let errors = pump_until(&mut rx, |rx| rx.state() == LinkState::Disconnected);
    assert!(matches!(errors[0], LinkError::Closed { code: None }), "{errors:?}");
}

#[test]
fn after_a_close_the_next_round_reconnects() {
    let (mut rx, mut peer, listener) = connected(config(), Fake::new());
    peer.frame(true, Opcode::Close, &[0x03, 0xE8]);
    pump_until(&mut rx, |rx| rx.state() == LinkState::Disconnected);

    let (rx, mut peer2) = handshake(rx.take(), &listener);
    let mut rx = rx;
    assert_eq!(rx.stats().opens, 2);

    peer2.text(SPOT_TRADE);
    pump_until(&mut rx, |rx| rx.pipeline().sink().len() == 1);
}

#[test]
fn a_half_message_does_not_survive_a_reconnect() {
    let (mut rx, mut peer, listener) = connected(config(), Fake::new());
    peer.frame(false, Opcode::Text, b"{\"half\":");
    pump_until(&mut rx, |rx| rx.stats().fragments == 1);
    peer.hang_up();
    pump_until(&mut rx, |rx| rx.state() == LinkState::Disconnected);

    let (mut rx, mut peer2) = handshake(rx.take(), &listener);
    // A continuation now is an orphan: the old message died with its socket.
    peer2.frame(true, Opcode::Continuation, b"1}");
    let errors = pump_until(&mut rx, |rx| rx.state() == LinkState::Disconnected);
    assert!(matches!(errors[0], LinkError::Protocol(WsError::UnexpectedContinuation)), "{errors:?}");
}

// ---------------------------------------------------------------------------
// Sending
// ---------------------------------------------------------------------------

#[test]
fn send_text_arrives_masked_and_reads_back() {
    let (mut rx, mut peer, _l) = connected(config(), Fake::new());
    let subscribe = br#"{"method":"SUBSCRIBE","params":["btcusdt@trade"],"id":1}"#;
    rx.send_text(subscribe).expect("connected");

    assert_eq!(peer.recv(), (Opcode::Text, subscribe.to_vec()));
}

#[test]
fn consecutive_frames_use_different_masks() {
    let (mut rx, mut peer, _l) = connected(config(), Fake::new());
    rx.send_text(b"a").unwrap();
    rx.send_text(b"b").unwrap();

    // Two masked one-byte frames: 2 header + 4 mask + 1 payload each.
    while peer.buf.len() < 14 {
        peer.fill();
    }
    let buf = peer.buf.clone();
    assert_ne!(&buf[2..6], &buf[9..13], "the mask moved");
    assert_eq!(peer.recv(), (Opcode::Text, b"a".to_vec()));
    assert_eq!(peer.recv(), (Opcode::Text, b"b".to_vec()));
}

#[test]
fn send_text_while_disconnected_says_so() {
    let mut rx = receiver(config(), 1, Fake::new());
    assert!(matches!(rx.send_text(b"x"), Err(SendError::Disconnected)));
}

#[test]
fn the_router_keepalive_is_sent_and_counted() {
    let (mut rx, mut peer, _l) = connected(config(), Fake::with_keepalive(1));
    pump_until(&mut rx, |rx| rx.stats().keepalives_sent >= 1);
    assert_eq!(peer.recv(), (Opcode::Text, KEEPALIVE.to_vec()));
}

#[test]
fn shutdown_sends_a_close() {
    let (mut rx, mut peer, _l) = connected(config(), Fake::new());
    rx.shutdown(CLOSE_GOING_AWAY);

    assert_eq!(rx.state(), LinkState::Disconnected);
    let (op, payload) = peer.recv();
    assert_eq!(op, Opcode::Close);
    assert_eq!(payload, CLOSE_GOING_AWAY.to_be_bytes());
}

// ---------------------------------------------------------------------------
// Silence
// ---------------------------------------------------------------------------

#[test]
fn silence_earns_a_ping_then_a_hangup() {
    let mut cfg = config();
    cfg.ping_interval_ns = 40_000_000;
    cfg.link.read_timeout = Duration::from_millis(5);
    let (mut rx, mut peer, _l) = connected(cfg, Fake::new());

    pump_until(&mut rx, |rx| rx.stats().pings_sent == 1);
    assert_eq!(peer.recv(), (Opcode::Ping, b"jeed".to_vec()));
    assert_eq!(rx.state(), LinkState::Open, "one interval is a question, not a verdict");

    let errors = pump_until(&mut rx, |rx| rx.state() == LinkState::Disconnected);
    assert!(matches!(errors[0], LinkError::Silent), "{errors:?}");
    assert_eq!(rx.stats().pings_sent, 1, "one ping per silence, not one per tick");
    let (op, payload) = peer.recv();
    assert_eq!(op, Opcode::Close);
    assert_eq!(payload, CLOSE_GOING_AWAY.to_be_bytes());
}

#[test]
fn any_frame_resets_the_silence_clock() {
    let mut cfg = config();
    cfg.ping_interval_ns = 40_000_000;
    cfg.link.read_timeout = Duration::from_millis(5);
    let (mut rx, mut peer, _l) = connected(cfg, Fake::new());

    // Data every 20 ms for 200 ms: five intervals, never two quiet ones.
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(200) {
        peer.text(br#"{"result":null,"id":1}"#);
        let until = Instant::now() + Duration::from_millis(20);
        while Instant::now() < until {
            rx.poll_once();
        }
    }
    assert_eq!(rx.state(), LinkState::Open);
    assert_eq!(rx.stats().pings_sent, 0);
    assert_eq!(rx.stats().disconnects, 0);
}

#[test]
fn a_pong_answers_the_ping_and_the_link_lives() {
    let mut cfg = config();
    cfg.ping_interval_ns = 40_000_000;
    cfg.link.read_timeout = Duration::from_millis(5);
    let (mut rx, mut peer, _l) = connected(cfg, Fake::new());

    pump_until(&mut rx, |rx| rx.stats().pings_sent == 1);
    let (op, payload) = peer.recv();
    assert_eq!(op, Opcode::Ping);
    peer.frame(true, Opcode::Pong, &payload);
    pump_until(&mut rx, |rx| rx.stats().pongs == 1);

    // Quiet again: a second ping, not a hangup.
    pump_until(&mut rx, |rx| rx.stats().pings_sent == 2);
    assert_eq!(rx.state(), LinkState::Open);
}

// ---------------------------------------------------------------------------
// The liveness record
// ---------------------------------------------------------------------------

#[test]
fn the_liveness_record_is_written_even_with_no_connection() {
    let mut cfg = config();
    cfg.record_heartbeat_ns = 1;
    let mut rx = receiver(cfg, 1, Fake::new());
    rx.poll_once();

    let rec = rx.pipeline().sink().last();
    assert_eq!(rec.kind(), Ok(WireKind::Heartbeat));
    assert_eq!(rx.stats().heartbeats, 1);
}

#[test]
fn the_liveness_record_keeps_its_cadence() {
    let mut cfg = config();
    cfg.record_heartbeat_ns = 1_000_000_000_000;
    let (mut rx, _peer, _l) = connected(cfg, Fake::new());
    for _ in 0..5 {
        rx.poll_once();
    }
    assert_eq!(rx.stats().heartbeats, 1, "one at the first round, the next in a thousand seconds");
}

#[test]
fn run_until_stops_and_closes() {
    let (rx, mut peer, _l) = connected(config(), Fake::new());
    let mut rx = rx;
    let rounds = std::cell::Cell::new(0);
    rx.run_until(
        || {
            rounds.set(rounds.get() + 1);
            rounds.get() > 3
        },
        |e| panic!("no errors expected: {e}"),
    );
    assert_eq!(rx.state(), LinkState::Disconnected);
    assert_eq!(peer.recv().0, Opcode::Close);
}
