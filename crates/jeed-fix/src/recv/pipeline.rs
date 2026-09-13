//! Everything the receive loop does that has nothing to do with the socket.
//!
//! ```text
//! [Frame] → 35 route ─┬─ W/X → decode → 34 verdict → symbol → adapt → publish
//!                     └─ admin → session state → what we owe the counterparty
//! ```
//!
//! **Transport-free, deliberately** — the same seam as
//! `jeed_krx::recv::pipeline`, for the same two reasons: the path to the ring
//! has to stay one path however the bytes arrived (`documents/todo.md` §15),
//! and the interesting half of a receive loop should be testable without a
//! network.
//!
//! Here it buys something extra. A FIX session is a conversation, and the
//! conversational logic — answer a `TestRequest`, ask back after a gap, hang up
//! on a regression — is all in [`ingest`](Pipeline::ingest), which returns a
//! [`Reply`] rather than writing one. The loop that owns the socket does the
//! writing. So the whole session protocol can be driven from a test with no
//! peer on the other end.
//!
//! ## `MdAdapter` is where the venue attaches
//!
//! KRX's pipeline ends at `dispatch::decode`, because a 전문 already says which
//! instrument it describes and in what units. A FIX message does not: a
//! `Symbol` is a venue's own string, an absent `MDEntrySize` means whatever the
//! venue says it means, and an incremental that touches the book may be
//! unmappable onto a wire format that has no delta record yet
//! (`documents/feed_handler.md` §13). All of that is venue knowledge, and this
//! crate has none — so the last step is a trait, and it is the only thing a
//! second FIX venue changes.
//!
//! ## Stale is decided here, not in the adapter
//!
//! Whether a reading is too old to act on is *handler policy* with a conf knob
//! behind it, and the consumer is told only the conclusion
//! (`documents/feed_handler.md` §8). An adapter that had to apply it could
//! forget, and then one venue would silently publish stale books. So the sink
//! the adapter is handed is a stale guard: every record it publishes passes
//! the age check on the way through, whatever the adapter does or does not do.
//!
//! ## Only one reply fits in an answer
//!
//! A message can owe two things at once — a `TestRequest` that also skipped a
//! sequence number wants a Heartbeat *and* a ResendRequest. [`Reply`] carries
//! one, and the order is Logout, then ResendRequest, then the rest: a
//! `TestRequest` is asking whether we are alive, and a ResendRequest is
//! traffic that answers that question while also asking the more urgent one.
//! An unanswered `TestRequest` is repeated; an unasked resend is not.

use crate::error::FixError;
use crate::frame::Frame;
use crate::market_data::{FixScales, FixText, MdMessage, parse_md_message};
use crate::recv::filter::SymbolFilter;
use crate::recv::stats::Stats;
use crate::session::{AdminMessage, SeqVerdict, SessionState, msg_type, parse_admin_message};
use crate::tagvalue::MsgType;
use jeed_wire::{
    HeartbeatPayload, RecordHeader, RecordSink, SYMBOL_LEN, UnixNano, Venue, WireKind, WireRecord,
    header_flags,
};

/// Turns decoded market-data messages into wire records. The venue layer.
///
/// Implemented outside this crate — `jeed-fix` knows FIX and nothing about any
/// venue that speaks it (`documents/feed_handler.md` §13).
pub trait MdAdapter {
    /// Why a message could not be mapped onto the wire.
    type Error;

    /// The venue every record carries. The counterpart of `jeed_krx::VENUE`,
    /// which this crate cannot have because FIX is a protocol, not a place.
    fn venue(&self) -> Venue;

    /// How this venue's prices and sizes become integers.
    ///
    /// A venue property, not a deployment choice: it is the precision USDKRW is
    /// quoted at, and getting it wrong is
    /// [`FixError::Precision`] rather than a
    /// rounded price.
    fn scales(&self) -> FixScales;

    /// Publishes the records `msg` implies and returns how many.
    ///
    /// Every record must be built **inside** `sink.publish` — building one
    /// elsewhere and handing it over copies 640 bytes for nothing, and loses
    /// the guarantee that a failed fill publishes nothing
    /// ([`RecordSink`]).
    ///
    /// **Refusing is sometimes the right answer.** A venue whose `35=X` moves
    /// the book has nothing to map onto a wire format with no delta record;
    /// dropping that quietly would leave the consumer's book wrong, so it
    /// returns an error and the loop counts it (§13).
    fn adapt<S: RecordSink>(
        &mut self,
        msg: &MdMessage,
        recv_ns: UnixNano,
        sink: &mut S,
    ) -> Result<usize, Self::Error>;
}

/// What the protocol owes the counterparty after one message.
///
/// Decided here, written by the loop that owns the socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    /// Answer a `TestRequest`, echoing its `112`.
    Heartbeat {
        /// `112` TestReqID to echo back. Empty when there was none.
        test_req_id: FixText,
    },

    /// Ask for `begin..=end` again. `end` of `0` is "everything from here".
    ResendRequest {
        /// First missing `MsgSeqNum`.
        begin: u64,

        /// Last missing one, or `0` for open-ended.
        end: u64,
    },

    /// Answer a `ResendRequest` we cannot satisfy.
    ///
    /// A market-data consumer sends nothing worth resending: every outbound
    /// message is session administration, and replaying a Heartbeat from ten
    /// minutes ago is worse than useless. `SequenceReset` with `GapFillFlag`
    /// is the protocol's way of saying exactly that.
    SequenceReset {
        /// The number the next real message will carry.
        new_seq: u64,
    },

    /// Hang up.
    Logout {
        /// `58` Text — why. A logout with no reason is indistinguishable from
        /// a crash in the venue's log.
        text: &'static str,
    },
}

/// What one framed message turned into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome<E> {
    /// Mapped onto the wire and published.
    Published {
        /// Records written. One message can be several — a `35=X` with three
        /// prints is three records.
        records: usize,
    },

    /// A session message, applied to [`SessionState`].
    Admin(MsgType),

    /// The symbol allow-set does not keep any instrument this message names.
    FilteredSymbol,

    /// A `PossDupFlag` resend of a number already seen. Dropped; it changes no
    /// state, which is the point of the flag.
    Duplicate,

    /// `MsgSeqNum` went backwards without `PossDupFlag`. Nothing was published
    /// and the session is being torn down.
    Regression,

    /// A `35=` this handler does not route. An `8` ExecutionReport here means
    /// the order session's credentials ended up in the feed handler's conf.
    Unhandled(MsgType),

    /// The decoder refused the message. Nothing was published.
    Failed(FixError),

    /// The venue adapter refused to map it. Nothing was published.
    Refused(E),
}

impl<E> Outcome<E> {
    /// `true` if a record reached the sink.
    #[inline]
    pub const fn published(&self) -> bool {
        matches!(self, Self::Published { .. })
    }
}

/// One message's effect: what it became, what its sequence number meant, and
/// what it leaves us owing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ingested<E> {
    /// What the message turned into.
    pub outcome: Outcome<E>,

    /// The sequence verdict, or `None` for a message that carried no usable
    /// `MsgSeqNum` — a `SequenceReset` with no `GapFillFlag`, whose own number
    /// the protocol says to ignore.
    pub verdict: Option<SeqVerdict>,

    /// What to send back, if anything.
    pub reply: Option<Reply>,

    /// `true` when the connection must be torn down after the reply is sent.
    pub disconnect: bool,
}

impl<E> Ingested<E> {
    /// An outcome that owes nothing and changes no session state.
    #[inline]
    const fn plain(outcome: Outcome<E>) -> Self {
        Self { outcome, verdict: None, reply: None, disconnect: false }
    }
}

/// Route, decode, adapt and publish — one message at a time.
#[derive(Debug)]
pub struct Pipeline<A, S> {
    symbols: SymbolFilter,
    stale_ns: u64,
    session: SessionState,
    stats: Stats,
    adapter: A,
    sink: S,
}

impl<A: MdAdapter, S: RecordSink> Pipeline<A, S> {
    /// Builds a pipeline.
    ///
    /// `stale_ns` is the age past which `recv_ns - venue_ns` earns the
    /// [`STALE`](header_flags::STALE) flag. **Zero disables the check**, which
    /// conf must say on purpose rather than reach by omission
    /// (`documents/feed_handler.md` §9).
    pub fn new(symbols: SymbolFilter, stale_ns: u64, adapter: A, sink: S) -> Self {
        Self {
            symbols,
            stale_ns,
            session: SessionState::new(),
            stats: Stats::default(),
            adapter,
            sink,
        }
    }

    /// Runs one framed message through the pipeline.
    ///
    /// `recv_ns` is stamped by the caller the instant the bytes came off the
    /// socket, not taken here — the clock read belongs next to the `read`.
    pub fn ingest(&mut self, frame: &Frame<'_>, recv_ns: UnixNano) -> Ingested<A::Error> {
        self.stats.received += 1;

        let kind = match msg_type(frame) {
            Ok(kind) => kind,
            Err(e) => return self.failed(e),
        };

        if kind.is_market_data() {
            self.market_data(frame, recv_ns)
        } else if kind.is_admin() {
            self.admin(frame, recv_ns)
        } else {
            // Not a fault: a session carrying an unexpected message type is a
            // configuration question, and one this handler cannot answer.
            self.stats.unhandled += 1;
            Ingested::plain(Outcome::Unhandled(kind))
        }
    }

    /// `35=W` / `35=X`.
    fn market_data(&mut self, frame: &Frame<'_>, recv_ns: UnixNano) -> Ingested<A::Error> {
        let msg = match parse_md_message(frame, self.adapter.scales()) {
            Ok(msg) => msg,
            Err(e) => return self.failed(e),
        };
        self.stats.market_data += 1;

        let verdict = self.session.observe(msg.msg_seq_num, msg.poss_dup, recv_ns);
        self.stats.note_verdict(verdict);
        let reply = match verdict {
            SeqVerdict::Duplicate { .. } => {
                return Ingested {
                    outcome: Outcome::Duplicate,
                    verdict: Some(verdict),
                    reply: None,
                    disconnect: false,
                };
            }
            SeqVerdict::Regression { .. } => {
                return Ingested {
                    outcome: Outcome::Regression,
                    verdict: Some(verdict),
                    reply: Some(Reply::Logout {
                        text: "MsgSeqNum went backwards without PossDupFlag",
                    }),
                    disconnect: true,
                };
            }
            // The message is here and describes a market that has moved.
            // Publishing it and asking for the hole are not alternatives.
            SeqVerdict::Gap { expected, received, .. } => {
                Some(Reply::ResendRequest { begin: expected, end: received - 1 })
            }
            SeqVerdict::InOrder => None,
        };

        if !self.symbols.allows_message(&msg) {
            self.stats.filtered_symbol += 1;
            return Ingested {
                outcome: Outcome::FilteredSymbol,
                verdict: Some(verdict),
                reply,
                disconnect: false,
            };
        }

        let mut stale = 0u64;
        let published = {
            let mut guard = StaleGuard {
                sink: &mut self.sink,
                recv_ns,
                stale_ns: self.stale_ns,
                stale: &mut stale,
            };
            self.adapter.adapt(&msg, recv_ns, &mut guard)
        };
        self.stats.stale += stale;

        let outcome = match published {
            Ok(records) => {
                self.stats.published += records as u64;
                Outcome::Published { records }
            }
            Err(e) => {
                // The slot, if one was claimed, was abandoned: a half-built
                // record is never what the consumer sees (`CLAUDE.md`).
                self.stats.refused += 1;
                self.sink.note_drops(1);
                Outcome::Refused(e)
            }
        };
        Ingested { outcome, verdict: Some(verdict), reply, disconnect: false }
    }

    /// `35` = `0 1 2 3 4 5 A`.
    fn admin(&mut self, frame: &Frame<'_>, recv_ns: UnixNano) -> Ingested<A::Error> {
        let msg = match parse_admin_message(frame) {
            Ok(msg) => msg,
            Err(e) => return self.failed(e),
        };
        self.stats.admin += 1;

        // A `SequenceReset` without `GapFillFlag` is administration, not a
        // message in the stream: the protocol says to apply it whatever number
        // it carries, so its own `34` is not judged.
        let administrative =
            matches!(msg.msg_type, MsgType::SequenceReset) && !msg.gap_fill;

        let verdict = if administrative {
            None
        } else {
            let v = self.session.observe(msg.msg_seq_num, msg.poss_dup, recv_ns);
            self.stats.note_verdict(v);
            Some(v)
        };

        self.session.apply_admin(&msg);
        if matches!(msg.msg_type, MsgType::Logon) {
            self.stats.logons += 1;
        }

        let (reply, disconnect) = self.answer(&msg, verdict);
        Ingested { outcome: Outcome::Admin(msg.msg_type), verdict, reply, disconnect }
    }

    /// What an administration message leaves us owing. See the module docs for
    /// why only one reply comes back.
    fn answer(
        &mut self,
        msg: &AdminMessage,
        verdict: Option<SeqVerdict>,
    ) -> (Option<Reply>, bool) {
        match verdict {
            Some(SeqVerdict::Regression { .. }) => {
                return (
                    Some(Reply::Logout {
                        text: "MsgSeqNum went backwards without PossDupFlag",
                    }),
                    true,
                );
            }
            Some(SeqVerdict::Duplicate { .. }) => return (None, false),
            Some(SeqVerdict::Gap { expected, received, .. }) => {
                return (Some(Reply::ResendRequest { begin: expected, end: received - 1 }), false);
            }
            _ => {}
        }

        match msg.msg_type {
            MsgType::TestRequest => {
                (Some(Reply::Heartbeat { test_req_id: msg.test_req_id }), false)
            }
            // We have nothing worth resending; say so rather than go quiet.
            MsgType::ResendRequest => (Some(Reply::SequenceReset { new_seq: 0 }), false),
            MsgType::Logout => (Some(Reply::Logout { text: "logout acknowledged" }), true),
            _ => (None, false),
        }
    }

    /// A message that did not decode. Nothing is published and the session
    /// survives — one malformed message is not a desynchronised stream, which
    /// is a framing error and handled a layer down.
    fn failed(&mut self, e: FixError) -> Ingested<A::Error> {
        self.stats.decode_failed += 1;
        self.sink.note_drops(1);
        Ingested::plain(Outcome::Failed(e))
    }

    /// Publishes a liveness record carrying the counters so far.
    ///
    /// Called from the receive loop itself and from nowhere else. A watchdog
    /// thread emitting this would keep reporting health after the loop it is
    /// watching had stopped, which is camouflage rather than monitoring
    /// (`documents/feed_handler.md` §9).
    pub fn heartbeat(&mut self, now: UnixNano) {
        let payload =
            HeartbeatPayload { received: self.stats.received, forwarded: self.stats.published };
        let header =
            RecordHeader::new(WireKind::Heartbeat, self.adapter.venue(), [0; SYMBOL_LEN], now);
        // Infallible: nothing in the closure can refuse.
        let filled = self.sink.publish(|rec| {
            *rec = WireRecord::new_heartbeat(header, payload);
            Ok::<(), core::convert::Infallible>(())
        });
        debug_assert!(filled.is_ok());
        self.stats.heartbeats += 1;
    }

    /// Starts a new session: the inbound sequence goes back to 1.
    ///
    /// The cumulative counters do **not** reset — a session that reconnects on
    /// every gap would otherwise show a clean sheet all afternoon
    /// ([`stats`](crate::recv::stats)).
    pub fn reset_session(&mut self) {
        self.session = SessionState::new();
    }

    /// Notes bytes read off the socket.
    #[inline]
    pub fn note_bytes(&mut self, n: u64) {
        self.stats.bytes += n;
    }

    /// Notes that the byte stream did not frame and the connection is going
    /// down with it.
    #[inline]
    pub fn note_framing_error(&mut self) {
        self.stats.framing_errors += 1;
    }

    /// Notes a connection lost or torn down.
    #[inline]
    pub fn note_disconnect(&mut self) {
        self.stats.disconnects += 1;
    }

    /// Notes a `connect` that failed.
    #[inline]
    pub fn note_connect_failure(&mut self) {
        self.stats.connect_failures += 1;
    }

    /// Notes an outbound message of the given shape.
    #[inline]
    pub fn note_sent(&mut self, reply: &Reply) {
        match reply {
            Reply::Heartbeat { .. } => self.stats.heartbeats_sent += 1,
            Reply::ResendRequest { .. } => self.stats.resend_requests_sent += 1,
            Reply::SequenceReset { .. } | Reply::Logout { .. } => {}
        }
    }

    /// Notes a `TestRequest` sent because the venue went quiet.
    #[inline]
    pub fn note_test_request(&mut self) {
        self.stats.test_requests_sent += 1;
    }

    /// Notes a heartbeat sent because *we* went quiet.
    #[inline]
    pub fn note_heartbeat_sent(&mut self) {
        self.stats.heartbeats_sent += 1;
    }

    /// Counters so far.
    #[inline]
    pub const fn stats(&self) -> &Stats {
        &self.stats
    }

    /// Sequence and liveness state of the current session.
    #[inline]
    pub const fn session(&self) -> &SessionState {
        &self.session
    }

    /// The same, for a handler resuming a session out of band.
    #[inline]
    pub const fn session_mut(&mut self) -> &mut SessionState {
        &mut self.session
    }

    /// The symbol allow-set, so an adapter can apply the same set per entry.
    #[inline]
    pub const fn symbols(&self) -> &SymbolFilter {
        &self.symbols
    }

    /// The venue adapter.
    #[inline]
    pub const fn adapter(&self) -> &A {
        &self.adapter
    }

    /// The venue adapter, mutably.
    #[inline]
    pub const fn adapter_mut(&mut self) -> &mut A {
        &mut self.adapter
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
/// The adapter never sees it and cannot opt out, which is the point: stale is
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
