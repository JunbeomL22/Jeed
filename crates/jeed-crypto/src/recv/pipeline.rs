//! Everything the receive loop does that has nothing to do with the socket.
//!
//! ```text
//! [text message] → Router::route → decoder → StaleGuard → RecordSink
//! ```
//!
//! **Transport-free, deliberately** — the same seam as
//! `jeed_krx::recv::pipeline` and `jeed_fix::recv::pipeline`, for the same
//! two reasons: the path to the ring has to stay one path however the bytes
//! arrived (`documents/todo.md` §15), and the interesting half of a receive
//! loop should be testable without a network.
//!
//! ## `Router` is where the venue attaches
//!
//! The decoders in this crate are per instrument and per channel: a frame is
//! decoded by [`binance::spot::trade::decode`](crate::binance::spot::trade::decode)
//! *because* the caller knows it came from `btcusdt@trade`. Which stream a
//! message belongs to is said differently by every venue — Binance's
//! combined-stream envelope, Bybit's `topic`, OKX's `arg`, KuCoin's `topic`
//! with a colon — and one connection can carry many. Getting from a message
//! to the right `(Instrument, decoder)` is therefore the venue layer, and it
//! is a trait here for the reason `jeed_fix::recv::MdAdapter` is: it is the
//! one thing that changes per venue, and the loop should not.
//!
//! The same trait carries the venue's *keepalive*. RFC 6455 pings are answered
//! by every server, but Bybit, OKX, Bitget, Gate and KuCoin each also want an
//! application-level ping in their own spelling, and a connection that does
//! not send it is dropped. That is venue conversation, so the router says
//! what to send and the loop sends it.
//!
//! ## Stale is decided here, not in the router
//!
//! Whether a reading is too old to act on is handler policy with a conf knob
//! behind it, and the consumer is told only the conclusion
//! (`documents/feed_handler.md` §8). A router that had to apply it could
//! forget. So the sink the router is handed is a guard: every record it
//! publishes passes the age check on the way through.

use crate::error::CryptoError;
use crate::recv::stats::Stats;
use jeed_wire::{
    HeartbeatPayload, RecordHeader, RecordSink, SYMBOL_LEN, UnixNano, Venue, WireKind, WireRecord,
    header_flags,
};

/// Longest venue keepalive the loop will send, in bytes.
///
/// `{"id":"1545910590801","type":"ping"}` is the longest any venue in this
/// crate asks for; 256 leaves room for one that wants a timestamp with it.
pub const MAX_KEEPALIVE_LEN: usize = 256;

/// Turns one text message into wire records. The venue layer.
pub trait Router {
    /// The venue every record on this connection carries. Stamped on the
    /// heartbeat record, which has no message to take it from.
    fn venue(&self) -> Venue;

    /// Publishes the records `msg` implies and returns how many.
    ///
    /// Zero is an ordinary answer — a subscription acknowledgement, a venue
    /// pong, a channel this handler did not ask for but was sent anyway.
    /// `Err` is not: the message was for a stream this router owns and the
    /// decoder refused it, and the loop counts it.
    ///
    /// Every record must be built **inside** `sink.publish` — building one
    /// elsewhere and handing it over copies 640 bytes for nothing, and loses
    /// the guarantee that a failed fill publishes nothing ([`RecordSink`]).
    fn route<S: RecordSink>(
        &mut self,
        msg: &[u8],
        recv_ns: UnixNano,
        sink: &mut S,
    ) -> Result<usize, CryptoError>;

    /// What the venue wants to hear so it does not hang up, if it is time.
    ///
    /// Called once per round. Writes a text payload into `out` and returns
    /// its length, or `None` when nothing is due. The default never sends
    /// anything, which is right for a venue that pings us (Binance, Upbit).
    fn keepalive(&mut self, now: UnixNano, out: &mut [u8; MAX_KEEPALIVE_LEN]) -> Option<usize> {
        let _ = (now, out);
        None
    }
}

/// What one message turned into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Routed, and `records` reached the sink — possibly zero.
    Published {
        /// Records written.
        records: usize,
    },

    /// The router refused it. Nothing was published.
    Failed(CryptoError),
}

impl Outcome {
    /// `true` if at least one record reached the sink.
    #[inline]
    pub const fn published(&self) -> bool {
        matches!(self, Self::Published { records } if *records > 0)
    }
}

/// Route, decode and publish — one message at a time.
#[derive(Debug)]
pub struct Pipeline<R, S> {
    stale_ns: u64,
    stats: Stats,
    router: R,
    sink: S,
}

impl<R: Router, S: RecordSink> Pipeline<R, S> {
    /// Builds a pipeline.
    ///
    /// `stale_ns` is the age past which `recv_ns - venue_ns` earns the
    /// [`STALE`](header_flags::STALE) flag. **Zero disables the check**, which
    /// conf must say on purpose rather than reach by omission
    /// (`documents/feed_handler.md` §9).
    pub fn new(stale_ns: u64, router: R, sink: S) -> Self {
        Self { stale_ns, stats: Stats::default(), router, sink }
    }

    /// Runs one text message through the router.
    ///
    /// `recv_ns` is stamped by the caller the instant the frame came off the
    /// socket, not taken here — the clock read belongs next to the `read`.
    pub fn ingest(&mut self, msg: &[u8], recv_ns: UnixNano) -> Outcome {
        self.stats.messages += 1;

        let mut stale = 0u64;
        let routed = {
            let mut guard = StaleGuard {
                sink: &mut self.sink,
                recv_ns,
                stale_ns: self.stale_ns,
                stale: &mut stale,
            };
            self.router.route(msg, recv_ns, &mut guard)
        };
        self.stats.stale += stale;

        match routed {
            Ok(records) => {
                self.stats.published += records as u64;
                Outcome::Published { records }
            }
            Err(e) => {
                // The slot, if one was claimed, was abandoned: a half-built
                // record is never what the consumer sees (`CLAUDE.md`).
                self.stats.decode_failed += 1;
                self.sink.note_drops(1);
                Outcome::Failed(e)
            }
        }
    }

    /// Publishes a liveness record carrying the counters so far.
    ///
    /// Called from the receive loop itself and from nowhere else. A watchdog
    /// thread emitting this would keep reporting health after the loop it is
    /// watching had stopped, which is camouflage rather than monitoring
    /// (`documents/feed_handler.md` §9).
    pub fn heartbeat(&mut self, now: UnixNano) {
        let payload =
            HeartbeatPayload { received: self.stats.messages, forwarded: self.stats.published };
        let header = RecordHeader::new(WireKind::Heartbeat, self.router.venue(), [0; SYMBOL_LEN], now);
        // Infallible: nothing in the closure can refuse.
        let filled = self.sink.publish(|rec| {
            *rec = WireRecord::new_heartbeat(header, payload);
            Ok::<(), core::convert::Infallible>(())
        });
        debug_assert!(filled.is_ok());
        self.stats.heartbeats += 1;
    }

    /// Notes bytes read off the socket.
    #[inline]
    pub fn note_bytes(&mut self, n: u64) {
        self.stats.bytes += n;
    }

    /// Notes a frame of any kind.
    #[inline]
    pub fn note_frame(&mut self) {
        self.stats.frames += 1;
    }

    /// Notes a frame that is part of a fragmented message.
    #[inline]
    pub fn note_fragment(&mut self) {
        self.stats.fragments += 1;
    }

    /// Notes a binary message, dropped.
    #[inline]
    pub fn note_binary(&mut self) {
        self.stats.binary += 1;
    }

    /// Notes a ping received.
    #[inline]
    pub fn note_ping(&mut self) {
        self.stats.pings += 1;
    }

    /// Notes a pong received.
    #[inline]
    pub fn note_pong(&mut self) {
        self.stats.pongs += 1;
    }

    /// Notes a Close frame received.
    #[inline]
    pub fn note_close(&mut self) {
        self.stats.closes += 1;
    }

    /// Notes a ping sent.
    #[inline]
    pub fn note_ping_sent(&mut self) {
        self.stats.pings_sent += 1;
    }

    /// Notes a pong sent.
    #[inline]
    pub fn note_pong_sent(&mut self) {
        self.stats.pongs_sent += 1;
    }

    /// Notes a venue keepalive sent.
    #[inline]
    pub fn note_keepalive(&mut self) {
        self.stats.keepalives_sent += 1;
    }

    /// Notes a frame that broke the protocol and took the connection with it.
    #[inline]
    pub fn note_protocol_error(&mut self) {
        self.stats.protocol_errors += 1;
    }

    /// Notes a connection lost or torn down.
    #[inline]
    pub fn note_disconnect(&mut self) {
        self.stats.disconnects += 1;
    }

    /// Notes a connect or handshake that failed.
    #[inline]
    pub fn note_connect_failure(&mut self) {
        self.stats.connect_failures += 1;
    }

    /// Notes a handshake completed.
    #[inline]
    pub fn note_open(&mut self) {
        self.stats.opens += 1;
    }

    /// Counters so far.
    #[inline]
    pub const fn stats(&self) -> &Stats {
        &self.stats
    }

    /// The venue router.
    #[inline]
    pub const fn router(&self) -> &R {
        &self.router
    }

    /// The venue router, mutably.
    #[inline]
    pub const fn router_mut(&mut self) -> &mut R {
        &mut self.router
    }

    /// The sink records are published to.
    #[inline]
    pub const fn sink(&self) -> &S {
        &self.sink
    }

    /// Gives the sink back.
    #[inline]
    pub fn into_sink(self) -> S {
        self.sink
    }
}

/// A [`RecordSink`] that applies the age check to everything on its way past.
///
/// The router never sees it and cannot opt out, which is the point: stale is
/// handler policy, and a policy one venue can forget to apply is not a policy
/// (see the module docs).
struct StaleGuard<'a, S> {
    sink: &'a mut S,
    recv_ns: UnixNano,
    stale_ns: u64,
    stale: &'a mut u64,
}

impl<S: RecordSink> RecordSink for StaleGuard<'_, S> {
    fn publish<E>(&mut self, fill: impl FnOnce(&mut WireRecord) -> Result<(), E>) -> Result<(), E> {
        let (recv_ns, stale_ns) = (self.recv_ns, self.stale_ns);
        let mut hit = false;
        let out = self.sink.publish(|rec| {
            fill(rec)?;
            hit = mark_stale(rec, recv_ns, stale_ns);
            Ok(())
        });
        if hit {
            *self.stale += 1;
        }
        out
    }

    #[inline]
    fn note_drops(&mut self, n: u64) {
        self.sink.note_drops(n);
    }
}

/// Flags a record whose exchange timestamp is older than `stale_ns`.
///
/// `saturating_sub` because [`UnixNano`] is unsigned and the two clocks are not
/// the same clock: a `venue_ns` ahead of `recv_ns` is skew, and skew must read
/// as "age zero", not as an age of eighteen billion years (`CLAUDE.md`).
#[inline]
fn mark_stale(rec: &mut WireRecord, recv_ns: UnixNano, stale_ns: u64) -> bool {
    if stale_ns == 0 || rec.header.flags & header_flags::VENUE_TIME_VALID == 0 {
        return false;
    }
    if recv_ns.saturating_sub(rec.header.venue_ns) <= stale_ns {
        return false;
    }
    rec.header.flags |= header_flags::STALE;
    true
}
