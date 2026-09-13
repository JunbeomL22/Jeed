//! Tests for `src/recv/receiver.rs` and `src/recv/link.rs` — the loop, against
//! a real socket.
//!
//! The venue is a `TcpListener` on loopback and the test itself plays it: it
//! reads what the handler sends and writes what a venue would. That is worth
//! more than a mock, because the thing being tested is exactly the part a mock
//! would stand in for — framing across arbitrary read boundaries, a connection
//! that goes away, a Logon that never comes back.
//!
//! Nothing here is Windows-only. `src/recv/link.rs` is `std::net`, not Winsock,
//! precisely so this can run anywhere.

use super::adapter::{FakeVenue, wire_symbol};
use super::builders::{HEARTBEAT, OPEN_SNAPSHOT, TWO_SIDED_SNAPSHOT, wrap};
use super::sink::Collect;
use jeed_fix::recv::{
    Config, Endpoint, Link, LinkState, Mode, SendError, SubscriptionRequest, SubscriptionType,
    SymbolFilter,
};
use jeed_fix::{Emitter, MdEntryType, MsgType, Receiver, frame, msg_type, parse_admin_message};
use jeed_wire::WireKind;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// The venue's end of the connection, plus whatever it has not framed yet.
struct Venue {
    peer: TcpStream,
    buf: Vec<u8>,
}

impl Venue {
    /// Reads one complete message the handler sent.
    fn recv(&mut self) -> Vec<u8> {
        loop {
            match frame(&self.buf) {
                Ok(f) => {
                    let msg = f.bytes.to_vec();
                    self.buf.drain(..msg.len());
                    return msg;
                }
                Err(jeed_fix::FixError::Incomplete) => {}
                Err(e) => panic!("the handler sent something unframeable: {e}"),
            }
            let mut chunk = [0u8; 4096];
            let n = self.peer.read(&mut chunk).expect("the handler should still be connected");
            assert!(n > 0, "the handler closed the connection");
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }

    /// Reads one message and says what type it was.
    fn recv_type(&mut self) -> MsgType {
        let raw = self.recv();
        msg_type(&frame(&raw).expect("frames")).expect("has 35=")
    }

    fn send(&mut self, body: &str) {
        self.peer.write_all(&wrap(body)).expect("write to the handler");
    }

    fn send_raw(&mut self, bytes: &[u8]) {
        self.peer.write_all(bytes).expect("write to the handler");
    }
}

type Rx = Receiver<FakeVenue, Collect>;

fn config() -> Config {
    Config {
        mode: Mode::Block,
        heartbeat_secs: 30,
        record_heartbeat_ns: 0,
        stale_ns: 0,
        reconnect_ns: 0,
        logon_timeout_ns: 60_000_000_000,
        link: jeed_fix::recv::LinkOptions {
            read_timeout: Duration::from_millis(20),
            ..Default::default()
        },
        ..Config::default()
    }
}

fn receiver(cfg: Config, port: u16) -> Rx {
    Receiver::new(
        cfg,
        Endpoint::new("127.0.0.1", port),
        Emitter::new(b"JEED", b"SMBS"),
        SymbolFilter::all(),
        FakeVenue::default(),
        Collect::new(),
    )
}

/// Brings up a listener, connects a receiver to it, and hands back both. The
/// handler's Logon has been sent but not answered.
fn connected(cfg: Config) -> (Rx, Venue, TcpListener) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().unwrap().port();
    let mut rx = receiver(cfg, port);

    assert!(rx.poll_once().is_none(), "connecting to a listening socket must succeed");
    assert_eq!(rx.state(), LinkState::LoggingOn);

    let (peer, _) = listener.accept().expect("accept the handler");
    peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    (rx, Venue { peer, buf: Vec::new() }, listener)
}

/// Runs rounds until `done` or the deadline, so a broken loop fails rather than
/// hangs.
fn pump_until(rx: &mut Rx, mut done: impl FnMut(&Rx) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done(rx) {
        assert!(Instant::now() < deadline, "the loop never got there");
        rx.poll_once();
    }
}

/// Logon, answered — with the interval we proposed, as a venue does.
fn logon(rx: &mut Rx, venue: &mut Venue) {
    let secs = rx.config().heartbeat_secs;
    let raw = venue.recv();
    let f = frame(&raw).expect("our Logon frames");
    let sent = parse_admin_message(&f).expect("our Logon decodes");
    assert_eq!(sent.msg_type, MsgType::Logon);
    assert_eq!(sent.msg_seq_num, 1);
    assert_eq!(sent.heartbeat_secs, Some(secs), "the Logon proposes the configured interval");

    logon_reply(rx, venue);
}

#[test]
fn the_loop_logs_on_and_publishes_what_the_venue_sends() {
    let (mut rx, mut venue, _l) = connected(config());
    logon(&mut rx, &mut venue);
    assert_eq!(rx.stats().logons, 1);
    assert_eq!(rx.session().heartbeat_interval_ns, 30_000_000_000, "as the venue named it");

    venue.send("35=W|49=SMBS|56=JEED|34=2|52=20260202-00:00:00.005|262=R|55=USDKRW|268=1|\
                269=1|270=1453.00|271=5000000|272=20260202|273=00:00:00.000");
    venue.send("35=W|49=SMBS|56=JEED|34=3|52=20260202-00:00:01.005|262=R|55=USDKRW|268=2|\
                269=0|270=1450.00|271=5000000|272=20260202|273=00:00:01.000|\
                269=1|270=1453.00|271=5000000|272=20260202|273=00:00:01.000");
    pump_until(&mut rx, |rx| rx.stats().published >= 2);

    assert_eq!(rx.stats().market_data, 2);
    assert_eq!(rx.stats().dropped(), 0);
    assert_eq!(rx.stats().lost, 0);
    assert_eq!(rx.session().next_expected, 4);

    let sink = rx.into_sink();
    assert_eq!(sink.len(), 2);
    assert_eq!(sink.records[1].kind(), Ok(WireKind::Quote));
    assert_eq!(sink.records[1].header.symbol, wire_symbol(b"USDKRW").unwrap());
    assert_eq!(sink.records[1].quote().unwrap().bid[0].price, 145_000);
}

#[test]
fn a_message_split_across_reads_is_reassembled() {
    // Where the boundary falls is the network's business, not the venue's.
    let (mut rx, mut venue, _l) = connected(config());
    logon(&mut rx, &mut venue);

    let raw = wrap(&OPEN_SNAPSHOT.replace("34=1|", "34=2|"));
    for chunk in raw.chunks(7) {
        venue.send_raw(chunk);
        rx.poll_once();
    }
    pump_until(&mut rx, |rx| rx.stats().published >= 1);
    assert_eq!(rx.stats().received, 2, "the Logon and one snapshot, not seven fragments");
}

#[test]
fn a_test_request_is_answered_on_the_wire() {
    let (mut rx, mut venue, _l) = connected(config());
    logon(&mut rx, &mut venue);

    venue.send("35=1|49=SMBS|56=JEED|34=2|52=20260202-00:00:00.000|112=ARE-YOU-THERE");
    pump_until(&mut rx, |rx| rx.stats().heartbeats_sent >= 1);

    let raw = venue.recv();
    let msg = parse_admin_message(&frame(&raw).unwrap()).expect("decodes");
    assert_eq!(msg.msg_type, MsgType::Heartbeat);
    assert_eq!(msg.test_req_id.as_bytes(), b"ARE-YOU-THERE", "the id comes back");
    assert_eq!(msg.msg_seq_num, 2, "and it occupies our next number");
}

#[test]
fn a_gap_is_asked_back_on_the_wire_and_the_message_is_still_published() {
    let (mut rx, mut venue, _l) = connected(config());
    logon(&mut rx, &mut venue);

    // 2, 3 and 4 never arrive.
    venue.send("35=W|49=SMBS|56=JEED|34=5|52=20260202-00:00:01.005|262=R|55=USDKRW|268=1|\
                269=1|270=1453.00|271=5000000|272=20260202|273=00:00:01.000");
    pump_until(&mut rx, |rx| rx.stats().resend_requests_sent >= 1);

    let raw = venue.recv();
    let msg = parse_admin_message(&frame(&raw).unwrap()).expect("decodes");
    assert_eq!(msg.msg_type, MsgType::ResendRequest);
    assert_eq!((msg.begin_seq_no, msg.end_seq_no), (Some(2), Some(4)), "exactly the hole");
    assert_eq!(rx.stats().published, 1, "and the message that exposed it was published");
    assert_eq!((rx.stats().gaps, rx.stats().lost), (1, 3));
}

#[test]
fn a_resend_request_we_cannot_satisfy_gets_a_gap_fill() {
    let (mut rx, mut venue, _l) = connected(config());
    logon(&mut rx, &mut venue);

    venue.send("35=2|49=SMBS|56=JEED|34=2|52=20260202-00:00:00.000|7=1|16=0");
    pump_until(&mut rx, |rx| rx.stats().admin >= 2);

    let raw = venue.recv();
    let msg = parse_admin_message(&frame(&raw).unwrap()).expect("decodes");
    assert_eq!(msg.msg_type, MsgType::SequenceReset);
    assert!(msg.gap_fill, "everything we send is administration; there is nothing to replay");
    assert!(msg.new_seq_no.unwrap() > msg.msg_seq_num);
}

#[test]
fn a_sequence_regression_ends_the_session_with_a_logout() {
    let (mut rx, mut venue, _l) = connected(config());
    logon(&mut rx, &mut venue);

    venue.send("35=W|49=SMBS|56=JEED|34=2|52=20260202-00:00:01.005|262=R|55=USDKRW|268=0");
    pump_until(&mut rx, |rx| rx.stats().received >= 2);
    // The same number again, without PossDupFlag.
    venue.send("35=W|49=SMBS|56=JEED|34=2|52=20260202-00:00:01.005|262=R|55=USDKRW|268=0");
    pump_until(&mut rx, |rx| rx.state() == LinkState::Disconnected);

    assert_eq!(rx.stats().regressions, 1);
    assert_eq!(rx.stats().disconnects, 1);
    let msg = parse_admin_message(&frame(&venue.recv()).unwrap()).expect("decodes");
    assert_eq!(msg.msg_type, MsgType::Logout, "the spec answers a regression with a logout");
}

#[test]
fn the_venue_hanging_up_disconnects_and_the_loop_dials_again() {
    // The ordinary afternoon. A loop that exited here would end the trading day
    // at the venue's first restart.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let mut rx = receiver(config(), port);

    rx.poll_once();
    let (peer, _) = listener.accept().expect("first connection");
    let mut venue = Venue { peer, buf: Vec::new() };
    peer_timeout(&venue);
    logon(&mut rx, &mut venue);

    drop(venue);
    pump_until(&mut rx, |rx| rx.state() == LinkState::Disconnected);
    assert_eq!(rx.stats().disconnects, 1);

    // `reconnect_ns` is zero in this config, so the next round dials again.
    pump_until(&mut rx, |rx| rx.state() == LinkState::LoggingOn);
    let (peer, _) = listener.accept().expect("second connection");
    let mut venue = Venue { peer, buf: Vec::new() };
    peer_timeout(&venue);

    let msg = parse_admin_message(&frame(&venue.recv()).unwrap()).expect("decodes");
    assert_eq!(msg.msg_type, MsgType::Logon);
    assert_eq!(msg.msg_seq_num, 1, "both directions start over on a new connection");
    logon_reply(&mut rx, &mut venue);
    assert_eq!(rx.stats().logons, 2);
    assert_eq!(rx.session().next_expected, 2, "the inbound sequence started over too");
}

#[test]
fn a_stream_that_does_not_frame_takes_the_connection_with_it() {
    // A BodyLength that lied makes every boundary after it a guess, so there is
    // nothing to resynchronise to.
    let (mut rx, mut venue, _l) = connected(config());
    logon(&mut rx, &mut venue);

    venue.send_raw(b"GET / HTTP/1.1\r\nHost: fix\r\n\r\n");
    pump_until(&mut rx, |rx| rx.state() == LinkState::Disconnected);
    assert_eq!(rx.stats().framing_errors, 1);
    assert_eq!(rx.stats().disconnects, 1);
}

#[test]
fn a_logon_that_is_never_answered_gives_up() {
    let cfg = Config { logon_timeout_ns: 50_000_000, ..config() };
    let (mut rx, mut venue, _l) = connected(cfg);
    assert_eq!(venue.recv_type(), MsgType::Logon);

    pump_until(&mut rx, |rx| rx.state() == LinkState::Disconnected);
    assert_eq!(rx.stats().disconnects, 1);
    assert_eq!(rx.stats().logons, 0);
}

#[test]
fn silence_earns_a_test_request_before_it_earns_a_teardown() {
    // One interval of nothing is a question, two are a verdict. The question is
    // what makes the verdict mean something: a venue that is alive answers.
    let cfg = Config { heartbeat_secs: 1, ..config() };
    let (mut rx, mut venue, _l) = connected(cfg);
    logon(&mut rx, &mut venue);

    pump_until(&mut rx, |rx| rx.stats().test_requests_sent >= 1);
    assert_eq!(rx.state(), LinkState::LoggedOn, "still up; we only asked");

    // Whatever we sent first, a TestRequest is among it.
    let mut seen = false;
    for _ in 0..4 {
        if venue.recv_type() == MsgType::TestRequest {
            seen = true;
            break;
        }
    }
    assert!(seen, "the venue should have been asked whether it is there");
}

#[test]
fn a_handler_with_no_venue_still_tells_the_consumer_it_is_alive() {
    // The consumer needs to hear "alive, no feed" rather than nothing at all,
    // which is what a crash sounds like.
    let cfg = Config {
        record_heartbeat_ns: 1,
        reconnect_ns: 60_000_000_000,
        ..config()
    };
    // Port 1 on loopback: nothing listens, and connect fails fast.
    let mut rx = receiver(cfg, 1);

    assert!(rx.poll_once().is_some(), "the failure is handed back for logging");
    assert_eq!(rx.state(), LinkState::Disconnected);
    assert_eq!(rx.stats().connect_failures, 1);

    rx.poll_once();
    let sink = rx.into_sink();
    assert!(sink.len() >= 2, "a liveness record per round, with no session at all");
    assert_eq!(sink.records[0].kind(), Ok(WireKind::Heartbeat));
}

#[test]
fn an_adopted_socket_still_gets_a_logon_from_the_loop() {
    // For a handler that obtained its socket elsewhere. The session protocol is
    // the loop's either way.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let (peer, _) = listener.accept().expect("accept");
    let mut venue = Venue { peer, buf: Vec::new() };
    peer_timeout(&venue);

    let mut rx = receiver(config(), port);
    rx.attach(Link::from_stream(stream).expect("wrap"), jeed_fix::clock::now_ns())
        .expect("logon sent");
    assert_eq!(rx.state(), LinkState::LoggingOn);
    assert_eq!(venue.recv_type(), MsgType::Logon);

    logon_reply(&mut rx, &mut venue);
    venue.send(TWO_SIDED_SNAPSHOT);
    pump_until(&mut rx, |rx| rx.stats().published >= 1);
}

#[test]
fn the_binary_subscribes_and_the_loop_never_does() {
    // What to ask for, at what depth, and whether this venue wants a request at
    // all is venue conversation rather than protocol obligation, so the loop
    // sends no 35=V of its own.
    let (mut rx, mut venue, _l) = connected(config());
    logon(&mut rx, &mut venue);

    let req = SubscriptionRequest {
        req_id: b"USDKRW-SMBS",
        subscription: SubscriptionType::SnapshotPlusUpdates,
        depth: 1,
        update_type: Some(b'1'),
        entry_types: &[MdEntryType::Bid, MdEntryType::Offer, MdEntryType::Trade],
        symbols: &[b"USDKRW"],
    };
    rx.subscribe(&req).expect("a logged-on session takes a subscription");

    let raw = venue.recv();
    assert_eq!(msg_type(&frame(&raw).unwrap()), Ok(MsgType::MarketDataRequest));
    let text = String::from_utf8_lossy(&raw).replace(jeed_fix::SOH as char, "|");
    assert!(text.contains("|262=USDKRW-SMBS|263=1|264=1|"));
    assert!(text.contains("|146=1|55=USDKRW|"));
    assert!(text.contains("|34=2|"), "it takes the next outbound number like anything else");

    // And it is refused rather than silently dropped when there is no session.
    rx.shutdown("done");
    assert!(matches!(rx.subscribe(&req), Err(SendError::Disconnected)));
}

#[test]
fn a_session_message_from_the_venue_is_not_a_published_record() {
    let (mut rx, mut venue, _l) = connected(config());
    logon(&mut rx, &mut venue);
    venue.send(&HEARTBEAT.replace("34=70", "34=2"));
    pump_until(&mut rx, |rx| rx.stats().admin >= 2);
    assert_eq!(rx.stats().published, 0);
    assert_eq!(rx.stats().dropped(), 0);
}

/// Answers a Logon that has already been read off the wire.
fn logon_reply(rx: &mut Rx, venue: &mut Venue) {
    let secs = rx.config().heartbeat_secs;
    venue.send(&format!("35=A|49=SMBS|56=JEED|34=1|52=20260202-00:00:00.000|98=0|108={secs}"));
    pump_until(rx, |rx| rx.state() == LinkState::LoggedOn);
}

fn peer_timeout(venue: &Venue) {
    venue.peer.set_read_timeout(Some(Duration::from_secs(5))).expect("set timeout");
}
