//! The receive loop: sockets on one side, one ring on the other.
//!
//! ```text
//!  ┌ pinned core ──────────────────────────────────────────────┐
//!  │  socket[0] ─┐                                             │
//!  │  socket[1] ─┼→ drain → Pipeline::ingest ──→ RecordSink    │
//!  │  socket[2] ─┘             │                               │
//!  │                           └→ heartbeat, same thread       │
//!  └───────────────────────────────────────────────────────────┘
//! ```
//!
//! **One pinned core = one receive thread = one producer = one ring.**
//! Splitting the hot feed across two cores would give it two producers, and the
//! ring is SPSC, so it would also give it two rings — with no router allowed to
//! merge them (`documents/feed_handler.md` §2). Until a measurement says one
//! core cannot drain the sockets, it stays one (`documents/todo.md` §5).
//!
//! ## spin and block are the same loop
//!
//! The two modes differ in one place: where the thread waits. [`Mode::Spin`]
//! never waits — every socket is polled every round and `WouldBlock` is the
//! normal answer — and costs a core. [`Mode::Block`] parks in the poll and
//! costs nothing. Everything after the datagram is identical, which is why
//! `V1`/`Q2` can arrive on a hot socket and still be treated as cold traffic:
//! the classification is the consumer's, off the record kind, not a wiring
//! decision here.
//!
//! ## The heartbeat is written by this loop or not at all
//!
//! A watchdog thread would keep publishing liveness after the receive loop had
//! stopped, which is camouflage rather than monitoring (§9). It is written
//! between rounds, from the same thread, so if this loop stops the heartbeat
//! stops with it.
//!
//! ## The burst cap is about the *other* sockets
//!
//! Draining a socket until it blocks is the cheapest way to empty it, but a
//! busy 선물 port would then hold the loop while the 콜/풋 ports queue. Each
//! socket yields at most [`Config::burst`] datagrams per round; whatever is
//! left is still there next round, one socket-visit later.

use crate::clock;
use crate::recv::endpoint::Endpoint;
use crate::recv::filter::{IsinFilter, TrCodeFilter};
use crate::recv::pipeline::{Outcome, Pipeline};
use crate::recv::socket::{FeedSocket, NetError, Poller, SocketOptions};
use crate::recv::stats::{SocketStats, Stats};
use jeed_wire::{RecordSink, UnixNano};

/// Read buffer for one datagram.
///
/// The longest interface KRX defines is 1387 B (REPO 우선호가) and the feed is
/// on a 1500 B MTU, so this is the next power of two above anything that can
/// arrive. A datagram larger than the buffer is truncated — reported as
/// `WSAEMSGSIZE` on Windows, silently on Linux — and either way it cannot be
/// half-decoded, because the length check rejects it.
pub const MAX_DATAGRAM: usize = 2048;

/// Where the receive thread waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Busy-spin over non-blocking sockets. Pins a core at 100 %, and the
    /// latency is one trip round the socket list.
    #[default]
    Spin,

    /// Park in the OS readiness wait. ~0 % CPU, and the latency is whatever it takes to
    /// wake the thread — tens of µs to ms, which is nothing next to a schedule
    /// notice arriving once a session.
    Block,
}

/// How a feed behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Where the thread waits.
    pub mode: Mode,

    /// Heartbeat interval. **Zero disables it**, which means the consumer can
    /// no longer tell a quiet market from a dead producer (§9).
    pub heartbeat_ns: u64,

    /// Age of `venue_ns` past which a record is flagged
    /// [`STALE`](jeed_wire::header_flags::STALE). **Zero disables it**, and
    /// disabling it once froze a book for twenty minutes (§9) — so the default
    /// here is on, and conf has to say `0` on purpose to turn it off.
    pub stale_ns: u64,

    /// Datagrams one socket may yield per round before the loop moves on.
    pub burst: u32,

    /// Socket setup. `nonblocking` is forced on: both modes drain with
    /// non-blocking reads, and [`Mode::Block`] waits in the poll instead.
    pub socket: SocketOptions,
}

impl Default for Config {
    /// spin, 100 ms heartbeat, 500 ms stale — the initial values in
    /// `conf/krx.example.toml`.
    fn default() -> Self {
        Self {
            mode: Mode::Spin,
            heartbeat_ns: 100_000_000,
            stale_ns: 500_000_000,
            burst: 64,
            socket: SocketOptions::default(),
        }
    }
}

impl Config {
    /// Readiness-wait timeout for [`Mode::Block`], in milliseconds.
    ///
    /// It is derived from the heartbeat rather than configured separately
    /// because it *is* the cold feed's tick: a socket with nothing on it wakes
    /// on the timeout, and that wake-up is when the heartbeat gets written. A
    /// second knob could only put the two out of step.
    #[inline]
    pub const fn poll_timeout_ms(&self) -> i32 {
        if self.heartbeat_ns == 0 {
            return 100;
        }
        let ms = self.heartbeat_ns / 1_000_000;
        if ms < 1 {
            1
        } else if ms > 1000 {
            1000
        } else {
            ms as i32
        }
    }
}

/// One feed: its sockets, its filters, and the single ring they publish to.
#[derive(Debug)]
pub struct Receiver<S> {
    cfg: Config,
    sockets: Vec<FeedSocket>,
    channels: Vec<SocketStats>,
    poller: Poller,
    pipeline: Pipeline<S>,
    next_heartbeat_ns: UnixNano,
    buf: [u8; MAX_DATAGRAM],
}

impl<S: RecordSink> Receiver<S> {
    /// Joins every endpoint and prepares the loop.
    ///
    /// Two things are validated, and one famously cannot be.
    ///
    /// The group must be a multicast address, and **no two endpoints may share
    /// a port**. That second rule is the stricter of the two platforms': Linux
    /// binds to the group and would demultiplex them correctly, Windows binds
    /// to `INADDR_ANY` and both sockets would receive both groups, publishing
    /// every message twice ([`socket`](crate::recv::socket)). Applying it
    /// everywhere keeps one conf valid on both, and a doubled book is not
    /// something to discover from the ring.
    ///
    /// **Which trcodes a socket carries cannot be checked here** — the
    /// assignment of content to ports is a circuit matter the distribution
    /// standard does not state — so a mis-assigned port shows up at runtime
    /// instead, as a configured trcode that never arrives
    /// ([`SocketStats::never_seen`]).
    pub fn new(
        cfg: Config,
        endpoints: &[Endpoint],
        trcodes: TrCodeFilter,
        isins: IsinFilter,
        sink: S,
    ) -> Result<Self, NetError> {
        let mut opts = cfg.socket;
        opts.nonblocking = true;

        let mut sockets = Vec::with_capacity(endpoints.len());
        let mut channels = Vec::with_capacity(endpoints.len());
        for (i, &endpoint) in endpoints.iter().enumerate() {
            if endpoints[..i].iter().any(|e| e.port == endpoint.port) {
                return Err(NetError { call: "duplicate port", endpoint, code: 0 });
            }
            sockets.push(FeedSocket::join(endpoint, opts)?);
            channels.push(SocketStats::new(endpoint, &trcodes));
        }

        let poller = Poller::new(&sockets);
        Ok(Self {
            cfg,
            sockets,
            channels,
            poller,
            pipeline: Pipeline::new(trcodes, isins, cfg.stale_ns, sink),
            // Zero arms the first heartbeat for the first round: the opening
            // record on the ring says the producer is alive before any market
            // data has arrived to say it implicitly.
            next_heartbeat_ns: 0,
            buf: [0; MAX_DATAGRAM],
        })
    }

    /// Runs until `stop` says otherwise.
    ///
    /// `stop` is checked once per round, so a cold feed exits within one
    /// [`poll_timeout_ms`](Config::poll_timeout_ms) of being asked to.
    pub fn run_until(&mut self, stop: impl Fn() -> bool) -> Result<(), NetError> {
        while !stop() {
            self.poll_once()?;
        }
        Ok(())
    }

    /// One round: visit the sockets, then write a heartbeat if one is due.
    pub fn poll_once(&mut self) -> Result<(), NetError> {
        let mut latest = None;

        match self.cfg.mode {
            Mode::Spin => {
                for i in 0..self.sockets.len() {
                    latest = self.drain(i)?.or(latest);
                }
            }
            Mode::Block => {
                let ready = self.poller.wait(self.cfg.poll_timeout_ms())?;
                if ready > 0 {
                    for i in 0..self.sockets.len() {
                        if self.poller.is_ready(i) {
                            latest = self.drain(i)?.or(latest);
                        }
                    }
                }
            }
        }

        // A round that received something already knows what time it is; only a
        // quiet round pays for a clock read, and a quiet round has time.
        let now = match latest {
            Some(ns) => ns,
            None => clock::now_ns(),
        };
        self.beat(now);
        Ok(())
    }

    /// Reads up to [`Config::burst`] datagrams off one socket.
    ///
    /// Returns the reception time of the last one, which the caller reuses as
    /// "now".
    fn drain(&mut self, index: usize) -> Result<Option<UnixNano>, NetError> {
        let mut latest = None;

        for _ in 0..self.cfg.burst {
            let n = match self.sockets[index].recv(&mut self.buf) {
                Ok(Some(n)) => n,
                // Empty. Every round on a spinning socket ends here.
                Ok(None) => break,
                Err(e) if e.is_transient() => {
                    self.channels[index].errors += 1;
                    self.pipeline.note_socket_error();
                    continue;
                }
                Err(e) => return Err(e),
            };

            let recv_ns = clock::now_ns();
            latest = Some(recv_ns);

            let outcome = self.pipeline.ingest(&self.buf[..n], recv_ns);

            let channel = &mut self.channels[index];
            channel.received += 1;
            channel.last_recv_ns = recv_ns;
            if let Outcome::Published { trcode } = outcome {
                channel.published += 1;
                // What this socket has actually carried — the only check on
                // the socket ↔ trcode wiring there is.
                channel.mark_seen(trcode);
            }
        }
        Ok(latest)
    }

    /// Writes a heartbeat if one is due.
    ///
    /// The next one is scheduled from `now`, not from the one that was due: a
    /// loop that fell behind should resume the cadence, not fire a burst of
    /// backdated liveness records catching up.
    fn beat(&mut self, now: UnixNano) {
        if self.cfg.heartbeat_ns == 0 || now < self.next_heartbeat_ns {
            return;
        }
        self.pipeline.heartbeat(now);
        self.next_heartbeat_ns = now.saturating_add(self.cfg.heartbeat_ns);
    }

    /// Counters for the feed as a whole.
    #[inline]
    pub const fn stats(&self) -> &Stats {
        self.pipeline.stats()
    }

    /// Per-socket counters, in the order the endpoints were given.
    #[inline]
    pub fn channels(&self) -> &[SocketStats] {
        &self.channels
    }

    /// The filter-decode-publish stage.
    #[inline]
    pub const fn pipeline(&self) -> &Pipeline<S> {
        &self.pipeline
    }

    /// The configuration this receiver was built with.
    #[inline]
    pub const fn config(&self) -> &Config {
        &self.cfg
    }

    /// Gives the sink back.
    #[inline]
    pub fn into_sink(self) -> S {
        self.pipeline.into_sink()
    }
}
