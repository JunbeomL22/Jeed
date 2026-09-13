//! Everything the receive loop does that has nothing to do with sockets.
//!
//! ```text
//! [bytes] → trcode → length → ISIN → claim slot → decode → stale → commit
//!            filter   check    filter                       flag
//! ```
//!
//! **Transport-free, deliberately.** `jeed-fix` reads a TCP stream and frames
//! it differently, but from "here are the bytes of one message" onward the two
//! handlers do the same things in the same order, and the path to the ring has
//! to be one path (`documents/todo.md` §15). Keeping that stretch out of the
//! socket loop is also what makes it testable: a test feeds it the same
//! builders the decoder tests use and never opens a socket.
//!
//! ## The order of the checks is the design
//!
//! Each stage is cheaper than the one it protects. The trcode lookup runs first
//! because most of what arrives on a 상품군 port is another data class
//! entirely; the length check runs before the ISIN read so a truncated datagram
//! cannot be indexed into; the ISIN check runs before a slot is claimed, so an
//! unwanted option strike never costs a decode. Only what survives all three
//! reaches shared memory.
//!
//! ## Stale is decided here, not in the decoder
//!
//! A decoder reports what the message said — including
//! [`VENUE_TIME_VALID`](jeed_wire::header_flags::VENUE_TIME_VALID) and the
//! assembled `venue_ns`. Whether that reading is too old to act on is a
//! *handler policy* with a conf knob behind it, and the consumer is told only
//! the conclusion (`documents/feed_handler.md` §8). The threshold defaults to
//! on: a conf without one once left a book frozen for twenty minutes because
//! the guard defaulted to disabled (§9).

use crate::VENUE;
use crate::decode::dispatch;
use crate::error::KrxError;
use crate::recv::filter::{IsinFilter, TrCodeFilter};
use crate::recv::stats::Stats;
use crate::trcode::TrCode;
use jeed_wire::{
    HeartbeatPayload, ISIN_LEN, RecordHeader, RecordSink, UnixNano, WireKind, WireRecord,
    header_flags,
};

/// What one datagram turned into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Published. The index is the trcode's position in the allow-set, which
    /// the caller uses to record what a socket has actually carried
    /// ([`SocketStats`](crate::recv::SocketStats)).
    Published {
        /// Position in the trcode allow-set.
        trcode: usize,
    },

    /// The trcode allow-set does not keep this code. The expected fate of most
    /// datagrams on a shared port.
    FilteredTrCode,

    /// The ISIN allow-set does not keep this instrument.
    FilteredIsin,

    /// Fewer than [`TRCODE_LEN`](crate::TRCODE_LEN) bytes.
    TooShort,

    /// Kept, but this build decodes no such code.
    UnknownTrCode(TrCode),

    /// Kept, and not the length its interface defines. Rejected before a slot
    /// was claimed.
    WrongLength {
        /// Length the interface defines.
        expected: usize,

        /// Length received.
        actual: usize,
    },

    /// Kept, and the decoder refused it. Nothing was published.
    Failed(KrxError),
}

impl Outcome {
    /// `true` if a record reached the sink.
    #[inline]
    pub const fn published(&self) -> bool {
        matches!(self, Self::Published { .. })
    }
}

/// Filter, decode and publish — one message at a time.
#[derive(Debug)]
pub struct Pipeline<S> {
    trcodes: TrCodeFilter,
    isins: IsinFilter,
    stale_ns: u64,
    stats: Stats,
    sink: S,
}

impl<S: RecordSink> Pipeline<S> {
    /// Builds a pipeline.
    ///
    /// `stale_ns` is the age past which `recv_ns - venue_ns` earns the
    /// [`STALE`](header_flags::STALE) flag. **Zero disables the check**, which
    /// conf must say explicitly rather than reach by omission (§9).
    pub fn new(trcodes: TrCodeFilter, isins: IsinFilter, stale_ns: u64, sink: S) -> Self {
        Self { trcodes, isins, stale_ns, stats: Stats::default(), sink }
    }

    /// Runs one datagram through the pipeline.
    ///
    /// `recv_ns` is stamped by the caller the instant the bytes came off the
    /// socket, not taken here — the clock read belongs next to the `recv`, not
    /// behind three branches of filtering.
    pub fn ingest(&mut self, payload: &[u8], recv_ns: UnixNano) -> Outcome {
        self.stats.received += 1;

        let Ok(trcode) = TrCode::from_message(payload) else {
            self.stats.too_short += 1;
            self.sink.note_drops(1);
            return Outcome::TooShort;
        };

        let Some(index) = self.trcodes.index_of(trcode) else {
            // Not a fault and not reported outward: this is the rest of the
            // product group's traffic, arriving exactly as expected.
            self.stats.filtered_trcode += 1;
            return Outcome::FilteredTrCode;
        };

        let Some(expected) = dispatch::message_len(trcode) else {
            self.stats.unknown_trcode += 1;
            self.sink.note_drops(1);
            return Outcome::UnknownTrCode(trcode);
        };

        if payload.len() != expected {
            self.stats.wrong_length += 1;
            self.sink.note_drops(1);
            return Outcome::WrongLength { expected, actual: payload.len() };
        }

        // Safe to index: the length now matches the interface, and every
        // interface's 종목코드 sits inside its own fixed header.
        if let Some(at) = dispatch::isin_offset(trcode)
            && !self.isins.allows(&payload[at..at + ISIN_LEN])
        {
            self.stats.filtered_isin += 1;
            return Outcome::FilteredIsin;
        }

        let stale_ns = self.stale_ns;
        let mut stale = false;
        let published = self.sink.publish(|rec| {
            dispatch::decode(payload, recv_ns, rec)?;
            stale = mark_stale(rec, recv_ns, stale_ns);
            Ok::<(), KrxError>(())
        });

        match published {
            Ok(()) => {
                self.stats.published += 1;
                self.stats.stale += u64::from(stale);
                Outcome::Published { trcode: index }
            }
            Err(e) => {
                // The slot was claimed and abandoned: a half-decoded record is
                // never what the consumer sees (`CLAUDE.md`).
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
    /// supposed to be watching had stopped, which is not monitoring but
    /// camouflage (§9).
    pub fn heartbeat(&mut self, now: UnixNano) {
        let payload =
            HeartbeatPayload { received: self.stats.received, forwarded: self.stats.published };
        let header = RecordHeader::new(WireKind::Heartbeat, VENUE, [0; ISIN_LEN], now);
        // Infallible: nothing in the closure can refuse.
        let filled = self.sink.publish(|rec| {
            *rec = WireRecord::new_heartbeat(header, payload);
            Ok::<(), core::convert::Infallible>(())
        });
        debug_assert!(filled.is_ok());
        self.stats.heartbeats += 1;
    }

    /// Notes a socket error the receive loop absorbed.
    ///
    /// It lives here rather than on the loop so one counter set describes the
    /// feed, whatever transport fed it.
    #[inline]
    pub fn note_socket_error(&mut self) {
        self.stats.socket_errors += 1;
    }

    /// Counters so far.
    #[inline]
    pub const fn stats(&self) -> &Stats {
        &self.stats
    }

    /// The trcode allow-set.
    #[inline]
    pub const fn trcodes(&self) -> &TrCodeFilter {
        &self.trcodes
    }

    /// The ISIN allow-set.
    #[inline]
    pub const fn isins(&self) -> &IsinFilter {
        &self.isins
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

/// Flags a decoded record whose exchange timestamp is older than `stale_ns`.
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
