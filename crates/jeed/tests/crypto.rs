//! `jeed::crypto` — the wiring, end to end: conf → rings → routers →
//! threads → a venue on loopback → records a consumer reads off the ring.
//!
//! The venue is a `TcpListener` the test plays: it completes the WebSocket
//! handshake, reads the subscriptions the router sent, answers with the
//! frames the decoder tests use (reached by path, not copied), and — for
//! Gate — serves the REST start book over plain HTTP on a second port. No
//! TLS: `ws://` and `http://` are the same code paths minus the handshake
//! the live test covers.

#[allow(dead_code)]
#[path = "../../jeed-crypto/tests/binance/common/mod.rs"]
mod binance_frames;
#[allow(dead_code)]
#[path = "../../jeed-crypto/tests/gate/common/mod.rs"]
mod gate_frames;

use core::sync::atomic::{AtomicBool, Ordering};
use jeed::conf::CryptoConf;
use jeed::crypto::{Options, StartError, start};
use jeed::toml::parse;
use jeed_crypto::recv::ws::{accept, Opcode};
use jeed_shm::{Recv, RingConsumer, SegmentName, SharedMapping};
use jeed_wire::{Venue, WireKind, WireRecord};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// The venue
// ---------------------------------------------------------------------------

/// A WebSocket server that, once the handler has subscribed, sends `frames`
/// and then holds the connection until the handler hangs up. Hands back
/// every text message the handler sent.
fn ws_venue(frames: Vec<Vec<u8>>, subscriptions_expected: usize) -> (u16, JoinHandle<Vec<Vec<u8>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();

        // Handshake.
        let mut buf = Vec::new();
        let head = read_until(&mut stream, &mut buf, b"\r\n\r\n");
        let text = String::from_utf8(head).unwrap();
        let key = text.lines().find_map(|l| l.strip_prefix("Sec-WebSocket-Key: ")).expect("a key");
        let key: [u8; 24] = key.as_bytes().try_into().unwrap();
        let mut reply = b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ".to_vec();
        reply.extend_from_slice(&accept(&key));
        reply.extend_from_slice(b"\r\n\r\n");
        stream.write_all(&reply).unwrap();

        // The subscriptions.
        let mut sent = Vec::new();
        while sent.len() < subscriptions_expected {
            let (op, payload) = read_client_frame(&mut stream, &mut buf);
            if op == Opcode::Text {
                sent.push(payload);
            }
        }

        // The market.
        for frame in frames {
            stream.write_all(&server_frame(Opcode::Text, &frame)).unwrap();
        }
        stream.flush().unwrap();

        // Hold until the handler closes; answer pings meanwhile.
        loop {
            match read_client_frame_opt(&mut stream, &mut buf) {
                Some((Opcode::Ping, payload)) => stream.write_all(&server_frame(Opcode::Pong, &payload)).unwrap(),
                Some((Opcode::Close, _)) | None => break,
                Some(_) => {}
            }
        }
        sent
    });
    (port, handle)
}

/// A one-request HTTP server answering `body` with a content length.
fn rest_venue(body: &'static [u8]) -> (u16, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut buf = Vec::new();
        let head = read_until(&mut stream, &mut buf, b"\r\n\r\n");
        let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len());
        stream.write_all(reply.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
        stream.flush().unwrap();
        String::from_utf8(head).unwrap()
    });
    (port, handle)
}

fn read_until(stream: &mut TcpStream, buf: &mut Vec<u8>, needle: &[u8]) -> Vec<u8> {
    loop {
        if let Some(i) = buf.windows(needle.len()).position(|w| w == needle) {
            return buf.drain(..i + needle.len()).collect();
        }
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).expect("the handler should still be connected");
        assert!(n > 0, "the handler closed the connection");
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn read_client_frame(stream: &mut TcpStream, buf: &mut Vec<u8>) -> (Opcode, Vec<u8>) {
    read_client_frame_opt(stream, buf).expect("the handler closed the connection")
}

/// One masked client frame, or `None` when the handler has hung up.
fn read_client_frame_opt(stream: &mut TcpStream, buf: &mut Vec<u8>) -> Option<(Opcode, Vec<u8>)> {
    loop {
        if let Some((op, payload, len)) = client_frame(buf) {
            buf.drain(..len);
            return Some((op, payload));
        }
        let mut chunk = [0u8; 4096];
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return None,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
}

fn client_frame(buf: &[u8]) -> Option<(Opcode, Vec<u8>, usize)> {
    if buf.len() < 2 {
        return None;
    }
    let opcode = Opcode::from_u8(buf[0] & 0x0F)?;
    assert!(buf[1] & 0x80 != 0, "a client frame must be masked");
    let (hlen, len) = match buf[1] & 0x7F {
        n if n < 126 => (2, n as usize),
        126 => {
            if buf.len() < 4 {
                return None;
            }
            (4, u16::from_be_bytes([buf[2], buf[3]]) as usize)
        }
        _ => {
            if buf.len() < 10 {
                return None;
            }
            (10, u64::from_be_bytes(buf[2..10].try_into().unwrap()) as usize)
        }
    };
    let total = hlen + 4 + len;
    if buf.len() < total {
        return None;
    }
    let mask = &buf[hlen..hlen + 4];
    let payload: Vec<u8> = buf[hlen + 4..total].iter().enumerate().map(|(i, b)| b ^ mask[i % 4]).collect();
    Some((opcode, payload, total))
}

fn server_frame(opcode: Opcode, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0x80 | opcode.as_u8()];
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

// ---------------------------------------------------------------------------
// The consumer
// ---------------------------------------------------------------------------

fn drain(rx: &mut RingConsumer) -> Vec<WireRecord> {
    let mut out = Vec::new();
    let mut rec = WireRecord::zeroed();
    loop {
        match rx.try_recv(&mut rec) {
            Recv::Record => out.push(rec),
            Recv::Empty => return out,
            Recv::Lagged(_) | Recv::Restarted { .. } => {}
        }
    }
}

fn wait_for(rx: &mut RingConsumer, into: &mut Vec<WireRecord>, done: impl Fn(&[WireRecord]) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        into.extend(drain(rx));
        if done(into) {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn kinds(records: &[WireRecord]) -> Vec<WireKind> {
    records.iter().filter_map(|r| r.kind().ok()).filter(|k| *k != WireKind::Heartbeat).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn two_feeds_subscribe_fetch_publish_and_stop() {
    let pid = std::process::id();

    // Binance: a combined-stream trade, once subscribed.
    let mut trade = br#"{"stream":"btcusdt@trade","data":"#.to_vec();
    trade.extend_from_slice(binance_frames::SPOT_TRADE);
    trade.push(b'}');
    let (binance_port, binance) = ws_venue(vec![trade], 1);

    // Gate: a start book over REST, then a diff on the socket. Two
    // subscription messages (trades, then the book channel).
    let (rest_port, rest) = rest_venue(gate_frames::REST_ORDER_BOOK);
    let (gate_port, gate) = ws_venue(vec![gate_frames::ORDER_BOOK_UPDATE.to_vec(), gate_frames::TRADE.to_vec()], 2);

    let text = format!(
        r#"
        [[feed]]
        name = "binance"
        venue = "binance-spot"
        mode = "block"
        cores = [0]
        ring = "jeed.test.{pid}.binance"
        ring_slots = 64
        url = "ws://127.0.0.1:{binance_port}/stream"

        [[feed.instrument]]
        symbol = "BTCUSDT"
        price_decimals = 2
        qty_decimals = 8
        channels = ["trade", "bbo"]

        [[feed]]
        name = "gate"
        venue = "gate-spot"
        mode = "block"
        cores = [1]
        ring = "jeed.test.{pid}.gate"
        ring_slots = 64
        url = "ws://127.0.0.1:{gate_port}/ws/v4/"
        rest = "http://127.0.0.1:{rest_port}"

        [[feed.instrument]]
        symbol = "BTC_USDT"
        price_decimals = 2
        qty_decimals = 4
        channels = ["trade", "delta"]

        [health]
        heartbeat_ms = 20
        stale_ms = 0
        "#
    );
    let conf = CryptoConf::from_table(&parse(&text).unwrap()).unwrap();
    conf.validate(None).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let opts = Options { boot_id: 0xc0de_0000_0000_0001 | u64::from(pid), pin: false, report_ns: 0 };
    let feeds = start(&conf, opts, Arc::clone(&stop)).unwrap();
    assert_eq!(feeds.len(), 2);

    let binance_ring = SegmentName::local(&format!("jeed.test.{pid}.binance")).unwrap();
    let gate_ring = SegmentName::local(&format!("jeed.test.{pid}.gate")).unwrap();
    let mut binance_rx = RingConsumer::attach(&binance_ring).unwrap();
    let mut gate_rx = RingConsumer::attach(&gate_ring).unwrap();
    assert_eq!(binance_rx.header().boot_id, opts.boot_id);

    let mut binance_records = Vec::new();
    wait_for(&mut binance_rx, &mut binance_records, |r| kinds(r).contains(&WireKind::Trade));
    let mut gate_records = Vec::new();
    wait_for(&mut gate_rx, &mut gate_records, |r| {
        let k = kinds(r);
        k.contains(&WireKind::Quote) && k.contains(&WireKind::SnapshotDelta) && k.contains(&WireKind::Trade)
    });

    stop.store(true, Ordering::Relaxed);
    for feed in feeds {
        let name = feed.name.clone();
        feed.join().unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    binance_records.extend(drain(&mut binance_rx));
    gate_records.extend(drain(&mut gate_rx));

    // Binance: subscribed by message, got its trade.
    let sent = binance.join().unwrap();
    assert_eq!(
        std::str::from_utf8(&sent[0]).unwrap(),
        r#"{"method":"SUBSCRIBE","params":["btcusdt@trade","btcusdt@bookTicker"],"id":1}"#
    );
    let trades: Vec<&WireRecord> = binance_records.iter().filter(|r| r.kind() == Ok(WireKind::Trade)).collect();
    assert_eq!(trades.len(), 1, "{:?}", kinds(&binance_records));
    assert_eq!(trades[0].header.venue(), Ok(Venue::BinanceSpot));
    assert_eq!(trades[0].header.symbol_bytes(), b"BTCUSDT");
    assert!(binance_records.iter().any(|r| r.kind() == Ok(WireKind::Heartbeat)), "heartbeats flow regardless");

    // Gate: two subscription messages, a REST GET with the id flag, and the
    // start book on the ring **as a Quote** beside the socket's records.
    let sent = gate.join().unwrap();
    assert_eq!(sent.len(), 2);
    assert!(std::str::from_utf8(&sent[0]).unwrap().contains(r#""channel":"spot.trades""#));
    assert!(std::str::from_utf8(&sent[1]).unwrap().contains(r#""payload":["BTC_USDT","100ms"]"#));
    let head = rest.join().unwrap();
    assert!(head.starts_with("GET /api/v4/spot/order_book?currency_pair=BTC_USDT&limit=10&with_id=true HTTP/1.1\r\n"), "{head}");
    let k = kinds(&gate_records);
    assert_eq!(k.iter().filter(|k| **k == WireKind::Quote).count(), 1, "{k:?}");
    assert_eq!(k.iter().filter(|k| **k == WireKind::SnapshotDelta).count(), 1, "{k:?}");
    assert_eq!(k.iter().filter(|k| **k == WireKind::Trade).count(), 1, "{k:?}");
    let quote = gate_records.iter().find(|r| r.kind() == Ok(WireKind::Quote)).unwrap();
    assert_eq!(quote.header.venue(), Ok(Venue::GateSpot));
    assert_eq!(quote.quote().unwrap().quote_ext, 48776300, "the REST book carries the id the diffs chain from");

    drop(binance_rx);
    drop(gate_rx);
    SharedMapping::unlink(&binance_ring).unwrap();
    SharedMapping::unlink(&gate_ring).unwrap();
}

#[test]
fn a_channel_the_venue_lacks_starts_nothing() {
    // The second feed asks Gate for a `book` channel; the first feed's ring,
    // already created, is dropped again and no thread runs.
    let pid = std::process::id();
    let text = format!(
        r#"
        [[feed]]
        name = "binance"
        venue = "binance-spot"
        mode = "block"
        cores = [0]
        ring = "jeed.test.{pid}.binance.bad"
        ring_slots = 64

        [[feed.instrument]]
        symbol = "BTCUSDT"
        price_decimals = 2
        qty_decimals = 8
        channels = ["trade"]

        [[feed]]
        name = "gate"
        venue = "gate-spot"
        mode = "block"
        cores = [1]
        ring = "jeed.test.{pid}.gate.bad"
        ring_slots = 64

        [[feed.instrument]]
        symbol = "BTC_USDT"
        price_decimals = 2
        qty_decimals = 4
        channels = ["book"]
        "#
    );
    let conf = CryptoConf::from_table(&parse(&text).unwrap()).unwrap();
    assert!(conf.validate(None).is_err(), "validate catches it first; start must too");

    let stop = Arc::new(AtomicBool::new(false));
    let opts = Options { boot_id: 7, pin: false, report_ns: 0 };
    let err = start(&conf, opts, stop).unwrap_err();
    assert!(matches!(&err, StartError::Router { feed, .. } if feed == "gate"), "{err}");

    let ring = SegmentName::local(&format!("jeed.test.{pid}.binance.bad")).unwrap();
    let _ = SharedMapping::unlink(&ring);
}
