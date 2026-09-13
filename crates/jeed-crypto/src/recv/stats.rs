//! Counters the receive loop keeps.
//!
//! Handler-internal, all of it. The consumer is told *conclusions*
//! (`STALE`), never the evidence (`documents/feed_handler.md` §8); what
//! crosses the boundary is the heartbeat record's two numbers, and those come
//! from here.
//!
//! ## What is not counted
//!
//! Loss. `jeed_fix::recv::Stats::lost` exists because `MsgSeqNum` is a
//! contiguous count the handler can hold the venue to. A WebSocket feed has
//! no such number: each venue's update-id chain is per stream, is carried
//! through to the consumer untouched, and only the consumer — the side with
//! a book — can say whether a hole in it matters (`CLAUDE.md`). So there is
//! nothing here for a message that never arrived, and
//! [`dropped`](Stats::dropped) counts only what did arrive and could not be
//! used.
//!
//! ## Counters outlive the connection
//!
//! Every reconnect starts a fresh WebSocket; none of these start over. A
//! feed that drops and redials on every venue restart would otherwise show a
//! clean sheet all afternoon.

/// What happened to every frame since boot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Bytes read off the socket, after TLS.
    pub bytes: u64,

    /// Frames of any kind framed off the stream.
    pub frames: u64,

    /// Text messages handed to the router — one per unfragmented text
    /// frame, one per reassembled fragmented message.
    pub messages: u64,

    /// Records published to the sink, excluding heartbeats. One message can
    /// be several — a batched trade frame is one record per print — or none,
    /// as a subscription acknowledgement is.
    pub published: u64,

    /// Published with [`STALE`](jeed_wire::header_flags::STALE) set.
    pub stale: u64,

    /// Heartbeat records published to the ring.
    pub heartbeats: u64,

    /// Messages the router refused. Nothing was published.
    pub decode_failed: u64,

    /// REST bodies handed to the router as start books
    /// ([`Pipeline::ingest_rest`](super::Pipeline::ingest_rest)). Not in
    /// `messages`: they did not come off the socket.
    pub rest: u64,

    /// Binary messages. Routed like text — Upbit and Bithumb send their JSON
    /// in binary frames — and counted here so a venue that starts sending
    /// them unexpectedly is visible.
    pub binary: u64,

    /// Frames that were part of a fragmented message, including the first.
    pub fragments: u64,

    /// Pings received, each answered.
    pub pings: u64,

    /// Pongs received — answers to ours, or unsolicited.
    pub pongs: u64,

    /// Close frames received.
    pub closes: u64,

    /// Pings sent after an interval of silence.
    pub pings_sent: u64,

    /// Pongs sent.
    pub pongs_sent: u64,

    /// Venue-level keepalives the router asked to send.
    pub keepalives_sent: u64,

    /// Frames that broke the protocol. Each one tore the connection down.
    pub protocol_errors: u64,

    /// Connections lost or torn down, for any reason.
    pub disconnects: u64,

    /// `connect` or handshake attempts that failed.
    pub connect_failures: u64,

    /// Handshakes completed. The binary resubscribes when this moves.
    pub opens: u64,
}

impl Stats {
    /// Messages this handler received and could not deliver.
    ///
    /// Binary frames are **not** in this number: nothing was asked of them.
    #[inline]
    pub const fn dropped(&self) -> u64 {
        self.decode_failed
    }
}
