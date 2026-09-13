//! The receive loop: one session on one side, one ring on the other.
//!
//! ```text
//!  ┌ pinned core ───────────────────────────────────────────────────┐
//!  │  connect → 35=A ──┐                                            │
//!  │                   ├→ read → frame → Pipeline::ingest → sink    │
//!  │  reconnect ←──────┤              │                             │
//!  │                   │              └→ Reply ──→ send             │
//!  │                   └→ silence: 35=1, then hang up               │
//!  └────────────────────────────────────────────────────────────────┘
//! ```
//!
//! **One pinned core = one receive thread = one producer = one ring**, exactly
//! as in `jeed_krx::recv::receiver` and for the same reason: the ring is SPSC,
//! so a second thread on this feed would need a second ring and nothing is
//! allowed to merge them (`documents/feed_handler.md` §2).
//!
//! ## What a KRX loop does not have to do
//!
//! Three things, all of them consequences of TCP being a conversation:
//!
//! **It logs on.** A multicast socket starts receiving the moment it joins. A
//! FIX session receives its first byte only after our `35=A`, so the loop has a
//! state ([`LinkState`]) where a KRX loop has none.
//!
//! **It answers.** Silence on a multicast port is a quiet market; silence on a
//! FIX session might be a dead one, and the only way to tell them apart is to
//! ask (`35=1`) and see whether anything comes back. Two intervals of nothing
//! and the session is dead by the protocol's own definition.
//!
//! **It reconnects.** A dropped multicast group is not a concept. A dropped TCP
//! session is the ordinary afternoon, and a loop that exited on one would end
//! the trading day at the venue's first restart. So a failure disconnects,
//! counts, and schedules another attempt — [`poll_once`](Receiver::poll_once)
//! hands the error back for logging rather than propagating it.
//!
//! ## Two heartbeats that are not the same heartbeat
//!
//! [`Config::heartbeat_secs`] is the FIX one: what we negotiate in the Logon
//! and send to the venue so it does not drop us.
//!
//! [`Config::record_heartbeat_ns`] is the wire one: a
//! [`WireKind::Heartbeat`](jeed_wire::WireKind::Heartbeat) record written to
//! the ring so the *consumer* can tell a quiet market from a dead producer
//! (§9). It is written by this loop between rounds and by nothing else — a
//! watchdog thread would keep reporting health after the loop it watches had
//! stopped, which is camouflage rather than monitoring.
//!
//! A session that is logged off still writes the second one. The consumer needs
//! to hear that the handler is alive and has no feed rather than hear nothing
//! at all, which is what a crash sounds like.

use crate::clock;
use crate::recv::emit::{EmitError, Emitter, MAX_EMIT_LEN, SubscriptionRequest};
use crate::recv::endpoint::Endpoint;
use crate::recv::filter::SymbolFilter;
use crate::recv::link::{Link, LinkError, LinkOptions};
use crate::recv::pipeline::{Ingested, MdAdapter, Outcome, Pipeline, Reply};
use crate::recv::stats::Stats;
use crate::session::SessionState;
use crate::tagvalue::MsgType;
use jeed_wire::{RecordSink, UnixNano};

/// Where the receive thread waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Busy-spin over a non-blocking socket. Pins a core at 100 %.
    ///
    /// Worth less here than on the KRX feed. There the spin saves the wake-up
    /// latency on every datagram of a 27-million-message day; a market-data
    /// session carries a fraction of that, and the kernel's wake-up is small
    /// next to the network hop that preceded it.
    Spin,

    /// Park in `read` until the timeout. ~0 % CPU, and the timeout is the tick
    /// that drives heartbeats and the silence check.
    #[default]
    Block,
}

/// Where the session is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinkState {
    /// No socket. A reconnect is scheduled.
    #[default]
    Disconnected,

    /// Connected, Logon sent, waiting for one back.
    LoggingOn,

    /// Logged on. Data may flow — after the binary has subscribed, if this
    /// venue needs a `35=V`.
    LoggedOn,
}

impl LinkState {
    /// `true` when a message can be sent.
    #[inline]
    pub const fn connected(self) -> bool {
        !matches!(self, Self::Disconnected)
    }
}

/// How a session behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Where the thread waits.
    pub mode: Mode,

    /// `108` HeartBtInt, in seconds. Sent in the Logon and used for the silence
    /// check when the venue's Logon does not name one of its own.
    ///
    /// **Zero disables the session's liveness entirely** — no heartbeats sent,
    /// no silence detected — which leaves a half-open TCP connection
    /// indistinguishable from a quiet market for as long as the OS keeps it.
    pub heartbeat_secs: u32,

    /// Send `141=Y` with the Logon.
    ///
    /// On by default: this handler keeps no book across a disconnect, so
    /// resuming a sequence only invites a resend of messages describing a
    /// market that has since moved.
    pub reset_seq_on_logon: bool,

    /// Cadence of the liveness record written to the ring. **Zero disables
    /// it**, and then the consumer cannot tell a quiet market from a dead
    /// producer (§9).
    pub record_heartbeat_ns: u64,

    /// Age of `venue_ns` past which a record is flagged
    /// [`STALE`](jeed_wire::header_flags::STALE). **Zero disables it**, and
    /// disabling it once froze a book for twenty minutes (§9).
    pub stale_ns: u64,

    /// Messages framed per round before the loop goes back to its other
    /// business.
    ///
    /// The cap is about the loop's *own* work, not about fairness between
    /// sockets as it is on the KRX side: there is one socket here, and what
    /// waits behind a long burst is the heartbeat and the silence check.
    pub burst: u32,

    /// Wait between connection attempts. Not exponential on purpose: a venue
    /// that is down comes back at a known time, and a backoff that has grown to
    /// minutes by then would keep the feed dark well past the opening.
    pub reconnect_ns: u64,

    /// How long to wait for the venue's Logon before giving up on the
    /// connection and starting over.
    pub logon_timeout_ns: u64,

    /// Socket setup.
    pub link: LinkOptions,
}

impl Default for Config {
    /// Block, 30 s FIX heartbeat, 100 ms record heartbeat, 500 ms stale, 5 s
    /// reconnect.
    fn default() -> Self {
        Self {
            mode: Mode::Block,
            heartbeat_secs: 30,
            reset_seq_on_logon: true,
            record_heartbeat_ns: 100_000_000,
            stale_ns: 500_000_000,
            burst: 64,
            reconnect_ns: 5_000_000_000,
            logon_timeout_ns: 10_000_000_000,
            link: LinkOptions::default(),
        }
    }
}

impl Config {
    /// The negotiated heartbeat interval in nanoseconds.
    #[inline]
    pub const fn heartbeat_ns(&self) -> u64 {
        self.heartbeat_secs as u64 * 1_000_000_000
    }
}

/// One market-data session: its socket, its filters, and the single ring they
/// publish to.
#[derive(Debug)]
pub struct Receiver<A, S> {
    cfg: Config,
    endpoint: Endpoint,
    emitter: Emitter,
    pipeline: Pipeline<A, S>,
    link: Option<Link>,
    state: LinkState,
    next_connect_ns: UnixNano,
    next_record_heartbeat_ns: UnixNano,
    logon_sent_ns: UnixNano,
    tested: bool,
    out: Box<[u8; MAX_EMIT_LEN]>,
}

impl<A: MdAdapter, S: RecordSink> Receiver<A, S> {
    /// Prepares a session. Nothing is connected until the first
    /// [`poll_once`](Self::poll_once).
    ///
    /// Connecting in the constructor would mean a handler that cannot start
    /// while the venue is down, and the venue is down every morning until it
    /// is not.
    pub fn new(
        cfg: Config,
        endpoint: Endpoint,
        emitter: Emitter,
        symbols: SymbolFilter,
        adapter: A,
        sink: S,
    ) -> Self {
        Self {
            cfg,
            endpoint,
            emitter,
            pipeline: Pipeline::new(symbols, cfg.stale_ns, adapter, sink),
            link: None,
            state: LinkState::Disconnected,
            // Zero arms the first connect and the first liveness record for the
            // first round: the opening record on the ring says the producer is
            // alive before any market data has arrived to say it implicitly.
            next_connect_ns: 0,
            next_record_heartbeat_ns: 0,
            logon_sent_ns: 0,
            tested: false,
            out: Box::new([0; MAX_EMIT_LEN]),
        }
    }

    /// Runs until `stop` says otherwise, handing every error to `on_error`.
    ///
    /// `stop` is checked once per round, so a quiet session exits within one
    /// read timeout of being asked to.
    pub fn run_until(&mut self, stop: impl Fn() -> bool, mut on_error: impl FnMut(LinkError)) {
        while !stop() {
            if let Some(e) = self.poll_once() {
                on_error(e);
            }
        }
        self.shutdown("handler stopping");
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
            _ => self.pump(now),
        };
        self.beat_record(clock::now_ns());
        failure
    }

    /// Sends a `35=V`.
    ///
    /// The loop never does this on its own: what to subscribe to, at what
    /// depth, and whether this venue wants a request at all are venue
    /// conversation rather than protocol obligation
    /// ([`SubscriptionRequest`]). The binary calls it once
    /// [`state`](Self::state) reaches [`LinkState::LoggedOn`], and again after
    /// every reconnect — [`Stats::logons`] counts those.
    pub fn subscribe(&mut self, req: &SubscriptionRequest<'_>) -> Result<(), SendError> {
        let now = clock::now_ns();
        let n = self.emitter.market_data_request(req, now, &mut self.out[..])?;
        let link = self.link.as_mut().ok_or(SendError::Disconnected)?;
        link.send(&self.out[..n]).map_err(SendError::Link)
    }

    /// Logs out and closes, if there is anything to close.
    pub fn shutdown(&mut self, reason: &'static str) {
        if self.link.is_some() {
            let _ = self.write(Reply::Logout { text: reason }, clock::now_ns());
        }
        self.drop_link();
    }

    /// Connects and sends the Logon, if the backoff has elapsed.
    fn try_connect(&mut self, now: UnixNano) -> Option<LinkError> {
        if now < self.next_connect_ns {
            return None;
        }
        self.next_connect_ns = now.saturating_add(self.cfg.reconnect_ns);

        let mut link = match Link::connect(&self.endpoint, self.cfg.link) {
            Ok(link) => link,
            Err(e) => {
                self.pipeline.note_connect_failure();
                return Some(e);
            }
        };
        if let Err(e) =
            link.set_mode(matches!(self.cfg.mode, Mode::Spin), self.cfg.link.read_timeout)
        {
            self.pipeline.note_connect_failure();
            return Some(e);
        }

        // Both directions start over. The venue is told so with `141=Y`, and
        // our own side is reset here — a Logon numbered 4831 against a venue
        // that expects 1 is a session that fails on its first message.
        self.emitter.reset();
        self.pipeline.reset_session();

        let n = match self.emitter.logon(
            self.cfg.heartbeat_secs,
            self.cfg.reset_seq_on_logon,
            now,
            &mut self.out[..],
        ) {
            Ok(n) => n,
            // A Logon that does not fit is a conf error, not a network one, and
            // retrying it will not help. It still goes through the reconnect
            // path so the loop keeps its shape.
            Err(_) => {
                self.pipeline.note_connect_failure();
                return Some(LinkError { call: "logon", source: None });
            }
        };
        if let Err(e) = link.send(&self.out[..n]) {
            self.pipeline.note_connect_failure();
            return Some(e);
        }

        self.link = Some(link);
        self.state = LinkState::LoggingOn;
        self.logon_sent_ns = now;
        self.tested = false;
        None
    }

    /// One round on a live connection.
    fn pump(&mut self, now: UnixNano) -> Option<LinkError> {
        match self.link.as_mut() {
            Some(link) => {
                let read = link.fill();
                match read {
                    Ok(n) => self.pipeline.note_bytes(n as u64),
                    Err(e) => return Some(self.fail(e)),
                }
            }
            None => {
                self.state = LinkState::Disconnected;
                return None;
            }
        }

        for _ in 0..self.cfg.burst {
            let recv_ns = clock::now_ns();

            // `self.link` and `self.pipeline` are disjoint fields, so the frame
            // can be borrowed out of one while the other decodes it — which is
            // what keeps the message from being copied out of the buffer.
            let step = {
                let Some(link) = self.link.as_ref() else { break };
                match link.peek() {
                    Ok(Some(frame)) => {
                        Some((frame.len(), self.pipeline.ingest(&frame, recv_ns)))
                    }
                    Ok(None) => None,
                    Err(_) => {
                        // Not one bad message: a `BodyLength` that lied makes
                        // every boundary after it a guess, and there is no
                        // resynchronising from that (`frame`).
                        self.pipeline.note_framing_error();
                        return Some(self.fail(LinkError { call: "framing", source: None }));
                    }
                }
            };
            let Some((len, ingested)) = step else { break };
            if let Some(link) = self.link.as_mut() {
                link.consume(len);
            }

            if let Some(e) = self.apply(ingested, now) {
                return Some(e);
            }
            if !self.state.connected() {
                return None;
            }
        }

        self.tick(now)
    }

    /// Applies one message's session effects and sends what it leaves us owing.
    fn apply(&mut self, ingested: Ingested<A::Error>, now: UnixNano) -> Option<LinkError> {
        if let Outcome::Admin(MsgType::Logon) = ingested.outcome {
            self.state = LinkState::LoggedOn;
            self.tested = false;
            // A venue that logs on without naming an interval leaves
            // `is_silent` permanently false, and the loop would never notice a
            // half-open socket. Ours is the interval we proposed.
            let session = self.pipeline.session_mut();
            if session.heartbeat_interval_ns == 0 {
                session.heartbeat_interval_ns = self.cfg.heartbeat_ns();
            }
        }

        if let Some(reply) = ingested.reply
            && let Err(e) = self.write(reply, now)
        {
            return Some(self.fail(e));
        }

        if ingested.disconnect {
            self.pipeline.note_disconnect();
            self.drop_link();
        }
        None
    }

    /// The between-messages business: logon timeout, our silence, their
    /// silence.
    fn tick(&mut self, now: UnixNano) -> Option<LinkError> {
        if matches!(self.state, LinkState::LoggingOn)
            && self.cfg.logon_timeout_ns != 0
            && now.saturating_sub(self.logon_sent_ns) > self.cfg.logon_timeout_ns
        {
            self.pipeline.note_disconnect();
            self.drop_link();
            return Some(LinkError { call: "logon timeout", source: None });
        }

        // Their silence. One interval earns a TestRequest, two mean the session
        // is dead by the protocol's own definition — and the TestRequest is
        // what makes the second interval mean something, because a venue that
        // is alive answers it.
        if self.pipeline.session().is_silent(now, 2) {
            self.pipeline.note_disconnect();
            self.shutdown("no response to TestRequest");
            return Some(LinkError { call: "peer silent", source: None });
        }
        if !self.tested && self.pipeline.session().is_silent(now, 1) {
            self.tested = true;
            let n = match self.emitter.test_request(b"JEED", now, &mut self.out[..]) {
                Ok(n) => n,
                Err(_) => return None,
            };
            self.pipeline.note_test_request();
            if let Some(link) = self.link.as_mut()
                && let Err(e) = link.send(&self.out[..n])
            {
                return Some(self.fail(e));
            }
        }

        // Our silence. The venue drops a session that goes quiet just as we do.
        if self.emitter.owes_heartbeat(now, self.cfg.heartbeat_ns()) {
            let n = match self.emitter.heartbeat(None, now, &mut self.out[..]) {
                Ok(n) => n,
                Err(_) => return None,
            };
            self.pipeline.note_heartbeat_sent();
            if let Some(link) = self.link.as_mut()
                && let Err(e) = link.send(&self.out[..n])
            {
                return Some(self.fail(e));
            }
        }
        None
    }

    /// Encodes a reply and writes it.
    fn write(&mut self, reply: Reply, now: UnixNano) -> Result<(), LinkError> {
        let encoded = match reply {
            Reply::Heartbeat { test_req_id } => {
                let id = test_req_id.as_bytes();
                let id = if id.is_empty() { None } else { Some(id) };
                self.emitter.heartbeat(id, now, &mut self.out[..])
            }
            Reply::ResendRequest { begin, end } => {
                self.emitter.resend_request(begin, end, now, &mut self.out[..])
            }
            // `new_seq` of 0 means "wherever we are" — the emitter knows its own
            // next number and nobody else should be guessing it.
            Reply::SequenceReset { new_seq } => {
                let target = if new_seq == 0 { self.emitter.next_seq() + 1 } else { new_seq };
                self.emitter.sequence_reset(target, true, now, &mut self.out[..])
            }
            Reply::Logout { text } => {
                self.emitter.logout(Some(text.as_bytes()), now, &mut self.out[..])
            }
        };
        // A reply that does not fit cannot be sent and cannot be fixed by
        // retrying; the session is better off without it than desynchronised.
        let Ok(n) = encoded else {
            return Ok(());
        };
        self.pipeline.note_sent(&reply);
        match self.link.as_mut() {
            Some(link) => link.send(&self.out[..n]),
            None => Ok(()),
        }
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
        self.state = LinkState::Disconnected;
        self.tested = false;
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

    /// Where the session is in its life.
    #[inline]
    pub const fn state(&self) -> LinkState {
        self.state
    }

    /// Counters for the session as a whole, across reconnects.
    #[inline]
    pub const fn stats(&self) -> &Stats {
        self.pipeline.stats()
    }

    /// Sequence and liveness state of the *current* session.
    #[inline]
    pub const fn session(&self) -> &SessionState {
        self.pipeline.session()
    }

    /// The address this session dials.
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
    pub const fn pipeline(&self) -> &Pipeline<A, S> {
        &self.pipeline
    }

    /// The outbound side, for a handler that wants to send something the loop
    /// does not.
    #[inline]
    pub const fn emitter_mut(&mut self) -> &mut Emitter {
        &mut self.emitter
    }

    /// Adopts an already-connected stream, for a test or a handler that
    /// obtained its socket elsewhere. The Logon is still sent by the loop.
    pub fn attach(&mut self, link: Link, now: UnixNano) -> Result<(), LinkError> {
        let mut link = link;
        link.set_mode(matches!(self.cfg.mode, Mode::Spin), self.cfg.link.read_timeout)?;
        self.emitter.reset();
        self.pipeline.reset_session();
        let n = self
            .emitter
            .logon(self.cfg.heartbeat_secs, self.cfg.reset_seq_on_logon, now, &mut self.out[..])
            .map_err(|_| LinkError { call: "logon", source: None })?;
        link.send(&self.out[..n])?;
        self.link = Some(link);
        self.state = LinkState::LoggingOn;
        self.logon_sent_ns = now;
        self.tested = false;
        Ok(())
    }

    /// Gives the sink back.
    #[inline]
    pub fn into_sink(self) -> S {
        self.pipeline.into_sink()
    }
}

/// A message could not be sent.
#[derive(Debug)]
pub enum SendError {
    /// There is no connection right now.
    Disconnected,

    /// The message did not fit the outbound buffer.
    TooLong(EmitError),

    /// The socket refused it.
    Link(LinkError),
}

impl From<EmitError> for SendError {
    fn from(e: EmitError) -> Self {
        Self::TooLong(e)
    }
}

impl core::fmt::Display for SendError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Disconnected => write!(f, "not connected"),
            Self::TooLong(e) => write!(f, "{e}"),
            Self::Link(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Disconnected => None,
            Self::TooLong(e) => Some(e),
            Self::Link(e) => Some(e),
        }
    }
}
