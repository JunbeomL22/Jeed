//! Counters the receive loop keeps.
//!
//! Handler-internal, all of it. The consumer is told *conclusions* (`STALE`),
//! never the evidence (`documents/feed_handler.md` §8); what crosses the
//! boundary is the heartbeat record's two numbers, and those come from here.
//!
//! ## Loss means something different here
//!
//! `jeed_krx::recv::stats` has no counter for missing messages, because on
//! multicast there is nothing to count against: 정보분배일련번호 arrives blank
//! on whole channels and is per (종목 × 보드) even when it does not
//! (`CLAUDE.md`), so the handler cannot know what it did not receive.
//!
//! TCP can. [`Stats::lost`] is the number of messages the counterparty sent and
//! we never saw, taken from `MsgSeqNum`, and on a healthy session it is exactly
//! zero for the whole day. A non-zero value is not a lossy market, it is a
//! session incident (§10) — which is why it is kept apart from
//! [`dropped`](Stats::dropped), the things we did receive and could not use.
//!
//! ## The sequence counters outlive the session
//!
//! [`SessionState`](crate::SessionState) resets to `next_expected = 1` on every
//! reconnect, as the protocol requires. These do not: a session that drops and
//! relogs on every gap would otherwise show a clean sheet all afternoon.

/// What happened to every message since boot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Bytes read off the socket.
    pub bytes: u64,

    /// Messages framed. Everything below adds up to this.
    pub received: u64,

    /// Records published to the sink, excluding heartbeats. One market-data
    /// message can produce several — a `35=X` with three prints is three
    /// records — so this is not a message count.
    pub published: u64,

    /// Published with [`STALE`](jeed_wire::header_flags::STALE) set.
    ///
    /// Counted per *record*, not per message: a snapshot that arrived late
    /// makes every level of it stale at once.
    pub stale: u64,

    /// Heartbeat records published to the ring.
    ///
    /// Not the same thing as [`heartbeats_sent`](Self::heartbeats_sent): one
    /// tells the consumer this handler is alive, the other tells the venue.
    pub heartbeats: u64,

    /// `35=W` / `35=X` messages decoded.
    pub market_data: u64,

    /// Session-administration messages (`35` = `0 1 2 3 4 5 A`) applied.
    pub admin: u64,

    /// Dropped by the symbol allow-set.
    pub filtered_symbol: u64,

    /// A `35=` this handler does not route — an `8` ExecutionReport on a
    /// market-data session, say. A configuration question, not a data fault.
    pub unhandled: u64,

    /// Framed, and the market-data decoder refused it. Nothing was published.
    pub decode_failed: u64,

    /// Decoded, and the venue adapter refused to map it onto the wire.
    ///
    /// Expected to be zero and important when it is not: an adapter refuses
    /// when publishing would make the consumer's book wrong (§13).
    pub refused: u64,

    /// The byte stream did not frame. Each one tore the connection down —
    /// there is no resynchronising a stream whose length field lied.
    pub framing_errors: u64,

    /// Gap events seen, across every session since boot.
    pub gaps: u64,

    /// Messages the counterparty sent that we never saw. Zero on a healthy
    /// session, all day.
    pub lost: u64,

    /// `PossDupFlag` resends dropped.
    pub duplicates: u64,

    /// Backwards jumps without `PossDupFlag` — a protocol violation, answered
    /// with a Logout rather than a skip.
    pub regressions: u64,

    /// Logons completed.
    pub logons: u64,

    /// Connections lost or torn down, for any reason.
    pub disconnects: u64,

    /// `connect` attempts that failed.
    pub connect_failures: u64,

    /// Heartbeats sent to the venue, including the ones answering a
    /// `TestRequest`.
    pub heartbeats_sent: u64,

    /// `TestRequest`s sent after one interval of silence.
    pub test_requests_sent: u64,

    /// `ResendRequest`s sent after a gap.
    pub resend_requests_sent: u64,
}

impl Stats {
    /// Messages this handler received and could not deliver.
    ///
    /// Filtered and unhandled traffic is **not** in this number: neither was
    /// something this handler was asked for. [`lost`](Self::lost) is not in it
    /// either — those never arrived at all.
    #[inline]
    pub const fn dropped(&self) -> u64 {
        self.decode_failed + self.refused
    }

    /// Messages a filter or the routing rejected.
    #[inline]
    pub const fn skipped(&self) -> u64 {
        self.filtered_symbol + self.unhandled
    }

    /// Folds a sequence verdict into the cumulative counters.
    #[inline]
    pub fn note_verdict(&mut self, verdict: crate::SeqVerdict) {
        match verdict {
            crate::SeqVerdict::InOrder => {}
            crate::SeqVerdict::Gap { missing, .. } => {
                self.gaps += 1;
                self.lost += missing;
            }
            crate::SeqVerdict::Duplicate { .. } => self.duplicates += 1,
            crate::SeqVerdict::Regression { .. } => self.regressions += 1,
        }
    }
}
