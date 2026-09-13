//! The receive loop: one WebSocket on one side, one ring on the other.
//!
//! ```text
//!  ┌ pinned core ──────────────────────────────────────────────────────┐
//!  │  connect → TLS → GET/101 ──┐                                      │
//!  │                            ├→ read → frame ─┬─ text → Pipeline    │
//!  │  reconnect ←───────────────┤                ├─ ping → pong        │
//!  │                            │                ├─ close → hang up    │
//!  │                            └→ silence: ping, then hang up         │
//!  └───────────────────────────────────────────────────────────────────┘
//! ```
//!
//! **One pinned core = one receive thread = one producer = one ring**, as in
//! the other two handlers and for the same reason: the ring is SPSC.
//!
//! ## What this loop shares with the FIX one, and what it does not
//!
//! Both are TCP conversations, so both connect, answer silence, and
//! reconnect ([`jeed_fix::recv::receiver`](https://docs.rs/jeed-fix) lays out
//! why each is needed). Two things differ:
//!
//! **There is no logon.** The WebSocket handshake *is* the session start,
//! and it completes inside [`Link::connect`] before the loop sees the
//! connection. So [`LinkState`] has two values, not three, and the moment
//! after a connect is the moment to subscribe — the binary watches
//! [`Stats::opens`] and calls [`send_text`](Receiver::send_text).
//!
//! **Liveness is two-layered.** RFC 6455 pings are answered by every server
//! and drive [`Config::ping_interval_ns`]: one quiet interval earns a ping,
//! two mean the peer is gone. But most venues *also* want an application
//! ping in their own dialect and drop a connection that does not send it;
//! that is [`Router::keepalive`], asked once per round.
//!
//! ## Two heartbeats that are not the same heartbeat
//!
//! The ping above keeps the venue from dropping us. The
//! [`WireKind::Heartbeat`](jeed_wire::WireKind::Heartbeat) record written
//! every [`Config::record_heartbeat_ns`] tells the *consumer* the producer is
//! alive — and it is written whether or not there is a connection, because
//! "alive with no feed" must sound different from a crash.

use crate::clock;
use crate::recv::endpoint::Endpoint;
use crate::recv::link::{
    CLOSE_GOING_AWAY, CLOSE_NORMAL, CLOSE_PROTOCOL_ERROR, Frame, Link, LinkError, LinkOptions, Tls,
    WS_MESSAGE_BUFFER,
};
use crate::recv::pipeline::{Failure, MAX_KEEPALIVE_LEN, Outcome, Pipeline, Router};
use crate::recv::stats::Stats;
use crate::recv::ws::{Assembly, MAX_CONTROL_PAYLOAD, Opcode};
use jeed_wire::{RecordSink, UnixNano};

/// Where the receive thread waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Busy-spin over a non-blocking socket. Pins a core at 100 %.
    Spin,

    /// Park in `read` until the timeout. ~0 % CPU, and the timeout is the tick
    /// that drives pings and the liveness record.
    #[default]
    Block,
}

/// Where the connection is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinkState {
    /// No socket. A reconnect is scheduled.
    #[default]
    Disconnected,

    /// Handshake done; frames may flow. Nothing has been subscribed unless
    /// the URL did it or the binary has sent something.
    Open,
}

impl LinkState {
    /// `true` when a message can be sent.
    #[inline]
    pub const fn connected(self) -> bool {
        matches!(self, Self::Open)
    }
}

/// How a connection behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Where the thread waits.
    pub mode: Mode,

    /// Quiet time before a ping is sent; twice this with nothing back and the
    /// peer is declared gone. Any frame counts as "something back", not only
    /// a pong — a data frame proves the peer is alive just as well.
    ///
    /// **Zero disables it**, and then a half-open socket is indistinguishable
    /// from a quiet market for as long as the OS keeps it.
    pub ping_interval_ns: u64,

    /// Cadence of the liveness record written to the ring. **Zero disables
    /// it**, and then the consumer cannot tell a quiet market from a dead
    /// producer (§9).
    pub record_heartbeat_ns: u64,

    /// Age of `venue_ns` past which a record is flagged
    /// [`STALE`](jeed_wire::header_flags::STALE). **Zero disables it.**
    pub stale_ns: u64,

    /// Frames handled per round before the loop goes back to its other
    /// business — the ping check, the keepalive, the liveness record.
    pub burst: u32,

    /// Wait between connection attempts. Not exponential on purpose: a venue
    /// that is down comes back at a known time, and a backoff that has grown
    /// to minutes by then keeps the feed dark past the moment it matters.
    pub reconnect_ns: u64,

    /// Socket and handshake setup.
    pub link: LinkOptions,
}

impl Default for Config {
    /// Block, 30 s ping, 100 ms record heartbeat, 500 ms stale, 5 s
    /// reconnect.
    fn default() -> Self {
        Self {
            mode: Mode::Block,
            ping_interval_ns: 30_000_000_000,
            record_heartbeat_ns: 100_000_000,
            stale_ns: 500_000_000,
            burst: 64,
            reconnect_ns: 5_000_000_000,
            link: LinkOptions::default(),
        }
    }
}

/// One WebSocket connection: its socket, its router, and the single ring they
/// publish to.
#[derive(Debug)]
pub struct Receiver<R, S> {
    cfg: Config,
    endpoint: Endpoint,
    tls: Tls,
    pipeline: Pipeline<R, S>,
    link: Option<Link>,
    assembly: Assembly,
    state: LinkState,
    next_connect_ns: UnixNano,
    next_record_heartbeat_ns: UnixNano,
    last_rx_ns: UnixNano,
    pinged: bool,
    keep: [u8; MAX_KEEPALIVE_LEN],
}

/// What the loop must do after a frame has been taken off the buffer.
enum Action {
    /// Nothing more.
    None,

    /// Answer a ping with its payload.
    Pong([u8; MAX_CONTROL_PAYLOAD], usize),

    /// The peer is hanging up.
    Close(Option<u16>),
}

impl<R: Router, S: RecordSink> Receiver<R, S> {
    /// Prepares a connection. Nothing is dialled until the first
    /// [`poll_once`](Self::poll_once).
    ///
    /// Connecting in the constructor would mean a handler that cannot start
    /// while the venue is down, and the venue is down every morning until it
    /// is not.
    pub fn new(cfg: Config, endpoint: Endpoint, tls: Tls, router: R, sink: S) -> Self {
        Self {
            cfg,
            endpoint,
            tls,
            pipeline: Pipeline::new(cfg.stale_ns, router, sink),
            link: None,
            assembly: Assembly::new(WS_MESSAGE_BUFFER),
            state: LinkState::Disconnected,
            // Zero arms the first connect and the first liveness record for
            // the first round.
            next_connect_ns: 0,
            next_record_heartbeat_ns: 0,
            last_rx_ns: 0,
            pinged: false,
            keep: [0; MAX_KEEPALIVE_LEN],
        }
    }

    /// Runs until `stop` says otherwise, handing every error to `on_error`.
    ///
    /// `stop` is checked once per round, so a quiet connection exits within
    /// one read timeout of being asked to.
    pub fn run_until(&mut self, stop: impl Fn() -> bool, mut on_error: impl FnMut(LinkError)) {
        while !stop() {
            if let Some(e) = self.poll_once() {
                on_error(e);
            }
        }
        self.shutdown(CLOSE_GOING_AWAY);
    }

    /// One round: connect or pump, then write a liveness record if one is due.
    ///
    /// Returns the error that ended the connection, **already handled** — the
    /// link is down, the counter is up, and a reconnect is scheduled. It comes
    /// back so the caller can log it, not so the caller can decide.
    pub fn poll_once(&mut self) -> Option<LinkError> {
        let now = clock::now_ns();
        let failure = match self.state {
            LinkState::Disconnected => self.try_connect(now),
            LinkState::Open => self.pump(now),
        };
        self.beat_record(clock::now_ns());
        failure
    }

    /// Sends a text frame — a subscription, in the venue's spelling.
    ///
    /// The loop never subscribes on its own: what to ask for and how to ask
    /// is venue conversation, and this crate has none. The binary calls this
    /// after every open — [`Stats::opens`] counts those.
    pub fn send_text(&mut self, payload: &[u8]) -> Result<(), SendError> {
        let link = self.link.as_mut().ok_or(SendError::Disconnected)?;
        match link.send_text(payload) {
            Ok(()) => Ok(()),
            Err(e) => Err(SendError::Link(self.fail(e))),
        }
    }

    /// Publishes a REST body as the start book for `instrument` — see
    /// [`Router::rest_books`].
    ///
    /// Goes through the same guarded sink as a socket message, so the age
    /// check and the counters apply. `recv_ns` is when the body arrived.
    pub fn ingest_rest(&mut self, instrument: usize, body: &[u8], recv_ns: UnixNano) -> Outcome {
        self.pipeline.ingest_rest(instrument, body, recv_ns)
    }

    /// The most recent message the router refused, once — see
    /// [`Pipeline::take_failure`].
    #[inline]
    pub fn take_failure(&mut self) -> Option<Failure> {
        self.pipeline.take_failure()
    }

    /// Replaces the URL the next connection attempt dials.
    ///
    /// For a venue whose address comes from a ticket ([`Router::ticket`]).
    /// The current connection, if any, is left alone.
    pub fn set_endpoint(&mut self, endpoint: Endpoint) {
        self.endpoint = endpoint;
    }

    /// Sends a Close and drops the connection, if there is one.
    pub fn shutdown(&mut self, code: u16) {
        if let Some(link) = self.link.as_mut() {
            let _ = link.send_close(code);
        }
        self.drop_link();
    }

    /// Dials, upgrades, and switches to the configured mode — if the backoff
    /// has elapsed.
    fn try_connect(&mut self, now: UnixNano) -> Option<LinkError> {
        if now < self.next_connect_ns {
            return None;
        }
        self.next_connect_ns = now.saturating_add(self.cfg.reconnect_ns);

        let mut link = match Link::connect(&self.endpoint, self.cfg.link, &self.tls) {
            Ok(link) => link,
            Err(e) => {
                self.pipeline.note_connect_failure();
                return Some(e);
            }
        };
        if let Err(e) = link.set_mode(matches!(self.cfg.mode, Mode::Spin), self.cfg.link.read_timeout)
        {
            self.pipeline.note_connect_failure();
            return Some(e);
        }

        self.assembly.reset();
        self.link = Some(link);
        self.state = LinkState::Open;
        self.last_rx_ns = now;
        self.pinged = false;
        self.pipeline.note_open();
        None
    }

    /// One round on a live connection.
    fn pump(&mut self, now: UnixNano) -> Option<LinkError> {
        match self.link.as_mut() {
            Some(link) => match link.fill() {
                Ok(n) => self.pipeline.note_bytes(n as u64),
                Err(e) => return Some(self.fail(e)),
            },
            None => {
                self.state = LinkState::Disconnected;
                return None;
            }
        }

        for _ in 0..self.cfg.burst {
            let recv_ns = clock::now_ns();

            // `link`, `pipeline` and `assembly` are disjoint fields, so the
            // frame can be borrowed out of one while the others consume it —
            // which is what keeps the payload from being copied out of the
            // buffer.
            let step = {
                let Some(link) = self.link.as_ref() else { break };
                match link.peek() {
                    Ok(Some(frame)) => {
                        let acted = on_frame(&mut self.pipeline, &mut self.assembly, &frame, recv_ns);
                        Some((frame.len(), acted))
                    }
                    Ok(None) => None,
                    Err(e) => {
                        self.pipeline.note_protocol_error();
                        return Some(self.fail(e));
                    }
                }
            };
            let Some((len, acted)) = step else { break };
            if let Some(link) = self.link.as_mut() {
                link.consume(len);
            }
            self.last_rx_ns = recv_ns;
            self.pinged = false;

            match acted {
                Ok(Action::None) => {}
                Ok(Action::Pong(payload, n)) => {
                    self.pipeline.note_pong_sent();
                    if let Some(link) = self.link.as_mut()
                        && let Err(e) = link.send_pong(&payload[..n])
                    {
                        return Some(self.fail(e));
                    }
                }
                Ok(Action::Close(code)) => {
                    // Echo the close as the protocol asks, then go. A venue
                    // that has said goodbye is not going to read it, and
                    // failing to send it is not a second error.
                    self.pipeline.note_disconnect();
                    self.shutdown(CLOSE_NORMAL);
                    return Some(LinkError::Closed { code });
                }
                Err(e) => {
                    self.pipeline.note_protocol_error();
                    self.pipeline.note_disconnect();
                    self.shutdown(CLOSE_PROTOCOL_ERROR);
                    return Some(e);
                }
            }
        }

        self.tick(now)
    }

    /// The between-frames business: their silence, and the venue's own ping.
    fn tick(&mut self, now: UnixNano) -> Option<LinkError> {
        let interval = self.cfg.ping_interval_ns;
        if interval != 0 {
            let quiet = now.saturating_sub(self.last_rx_ns);
            if quiet > interval.saturating_mul(2) {
                self.pipeline.note_disconnect();
                self.shutdown(CLOSE_GOING_AWAY);
                return Some(LinkError::Silent);
            }
            if !self.pinged && quiet > interval {
                self.pinged = true;
                self.pipeline.note_ping_sent();
                if let Some(link) = self.link.as_mut()
                    && let Err(e) = link.send_ping(b"jeed")
                {
                    return Some(self.fail(e));
                }
            }
        }

        if let Some(n) = self.pipeline.router_mut().keepalive(now, &mut self.keep) {
            self.pipeline.note_keepalive();
            if let Some(link) = self.link.as_mut()
                && let Err(e) = link.send_text(&self.keep[..n])
            {
                return Some(self.fail(e));
            }
        }
        None
    }

    /// Counts a failure, drops the connection, and hands the error back.
    fn fail(&mut self, e: LinkError) -> LinkError {
        self.pipeline.note_disconnect();
        self.drop_link();
        e
    }

    /// Closes the socket and arms the reconnect.
    fn drop_link(&mut self) {
        self.link = None;
        self.assembly.reset();
        self.state = LinkState::Disconnected;
        self.pinged = false;
    }

    /// Writes a liveness record to the ring if one is due.
    ///
    /// Scheduled from `now` rather than from the one that was due: a loop that
    /// fell behind should resume the cadence, not fire a burst of backdated
    /// liveness records catching up.
    fn beat_record(&mut self, now: UnixNano) {
        if self.cfg.record_heartbeat_ns == 0 || now < self.next_record_heartbeat_ns {
            return;
        }
        self.pipeline.heartbeat(now);
        self.next_record_heartbeat_ns = now.saturating_add(self.cfg.record_heartbeat_ns);
    }

    /// Where the connection is in its life.
    #[inline]
    pub const fn state(&self) -> LinkState {
        self.state
    }

    /// Counters for the feed as a whole, across reconnects.
    #[inline]
    pub const fn stats(&self) -> &Stats {
        self.pipeline.stats()
    }

    /// The URL this loop dials.
    #[inline]
    pub const fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// The configuration this receiver was built with.
    #[inline]
    pub const fn config(&self) -> &Config {
        &self.cfg
    }

    /// The route-decode-publish stage.
    #[inline]
    pub const fn pipeline(&self) -> &Pipeline<R, S> {
        &self.pipeline
    }

    /// The venue router, for a binary that changes what it wants mid-day.
    #[inline]
    pub const fn router_mut(&mut self) -> &mut R {
        self.pipeline.router_mut()
    }

    /// Gives the sink back.
    #[inline]
    pub fn into_sink(self) -> S {
        self.pipeline.into_sink()
    }
}

/// Takes one frame off the buffer: data to the pipeline, control to the loop.
///
/// `recv_ns` is the stamp of the frame that *completed* a message — for a
/// fragmented one, its last fragment. That is when the bytes became a message.
fn on_frame<R: Router, S: RecordSink>(
    pipeline: &mut Pipeline<R, S>,
    assembly: &mut Assembly,
    frame: &Frame<'_>,
    recv_ns: UnixNano,
) -> Result<Action, LinkError> {
    pipeline.note_frame();
    match frame.opcode {
        Opcode::Ping => {
            pipeline.note_ping();
            let mut payload = [0u8; MAX_CONTROL_PAYLOAD];
            let n = frame.payload.len().min(MAX_CONTROL_PAYLOAD);
            payload[..n].copy_from_slice(&frame.payload[..n]);
            Ok(Action::Pong(payload, n))
        }
        Opcode::Pong => {
            pipeline.note_pong();
            Ok(Action::None)
        }
        Opcode::Close => {
            pipeline.note_close();
            Ok(Action::Close(frame.close_code()))
        }
        Opcode::Text | Opcode::Binary => {
            if frame.fin && !assembly.is_open() {
                ingest(pipeline, frame.opcode, frame.payload, recv_ns);
            } else {
                assembly.begin(frame.opcode, frame.payload)?;
                pipeline.note_fragment();
            }
            Ok(Action::None)
        }
        Opcode::Continuation => {
            assembly.append(frame.payload)?;
            pipeline.note_fragment();
            if frame.fin && let Some((opcode, bytes)) = assembly.take() {
                ingest(pipeline, opcode, bytes, recv_ns);
            }
            Ok(Action::None)
        }
    }
}

/// Text and binary both go to the router; binary is counted on the way.
///
/// Upbit and Bithumb send their JSON in binary frames, so a loop that dropped
/// them would be a loop that could not carry two of its eight venues. The
/// counter stays so a venue that *starts* sending binary is visible.
fn ingest<R: Router, S: RecordSink>(
    pipeline: &mut Pipeline<R, S>,
    opcode: Opcode,
    bytes: &[u8],
    recv_ns: UnixNano,
) {
    if matches!(opcode, Opcode::Binary) {
        pipeline.note_binary();
    }
    pipeline.ingest(bytes, recv_ns);
}

/// A message could not be sent.
#[derive(Debug)]
pub enum SendError {
    /// There is no connection right now.
    Disconnected,

    /// The socket refused it. The connection has been dropped.
    Link(LinkError),
}

impl core::fmt::Display for SendError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Disconnected => write!(f, "not connected"),
            Self::Link(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Disconnected => None,
            Self::Link(e) => Some(e),
        }
    }
}
