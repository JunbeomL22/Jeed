//! Counters the receive loop keeps.
//!
//! All of it is handler-internal and none of it reaches the wire: the consumer
//! is told *conclusions* (`STALE`), never the evidence
//! (`documents/feed_handler.md` §8). What crosses the boundary is the
//! heartbeat's two numbers, and those come from here.
//!
//! ## Drops are counted twice on purpose
//!
//! [`Stats::dropped`] is everything the loop refused to publish. The segment's
//! own drop counter (`RingProducer::note_drops`) is deliberately *not* the same
//! number: a datagram the trcode filter rejected was never a record this
//! handler was asked for, and rolling it into the shared counter would bury a
//! real decode failure under the other product's traffic on the same port.
//! Only the messages this handler *wanted* and could not deliver are reported
//! outward.

use crate::recv::endpoint::Endpoint;
use crate::recv::filter::TrCodeFilter;
use jeed_wire::UnixNano;

/// What happened to every datagram since boot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Datagrams read off a socket.
    pub received: u64,

    /// Records published to the sink, excluding heartbeats.
    pub published: u64,

    /// Heartbeat records published.
    pub heartbeats: u64,

    /// Published with [`STALE`](jeed_wire::header_flags::STALE) set.
    pub stale: u64,

    /// Dropped by the trcode allow-set — the configured-away traffic that
    /// shares the port. Expected to dwarf everything else here.
    pub filtered_trcode: u64,

    /// Dropped by the ISIN allow-set.
    pub filtered_isin: u64,

    /// Shorter than a trcode. A datagram this short is not a KRX message.
    pub too_short: u64,

    /// Kept by the filters, but this build has no decoder for the trcode. A
    /// configuration question (why is that channel joined?), not a data fault.
    pub unknown_trcode: u64,

    /// Kept by the filters and the wrong length for its interface. Caught
    /// before a slot is claimed, so it costs one comparison.
    pub wrong_length: u64,

    /// Kept by the filters and failed to decode. Nothing was published.
    pub decode_failed: u64,

    /// Socket-level errors the loop absorbed rather than failed on.
    pub socket_errors: u64,
}

impl Stats {
    /// Datagrams this handler wanted and could not deliver.
    ///
    /// Filtered traffic is **not** in this number — see the module docs.
    #[inline]
    pub const fn dropped(&self) -> u64 {
        self.too_short + self.unknown_trcode + self.wrong_length + self.decode_failed
    }

    /// Datagrams a filter rejected.
    #[inline]
    pub const fn filtered(&self) -> u64 {
        self.filtered_trcode + self.filtered_isin
    }
}

/// Per-socket counters, and the record of which configured trcodes this socket
/// has actually carried.
///
/// **Startup cannot validate the socket ↔ trcode mapping** — which port carries
/// what is a circuit assignment, not something the distribution standard says
/// (`documents/todo.md` §5). [`never_seen`](Self::never_seen) is the runtime
/// substitute: a code configured on a socket that has never delivered one is
/// how a mis-assigned port becomes visible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketStats {
    /// The endpoint this socket joined.
    pub endpoint: Endpoint,

    /// Datagrams read off this socket.
    pub received: u64,

    /// Records published from this socket's datagrams.
    pub published: u64,

    /// Reception time of the last datagram, or `0` if none has arrived.
    ///
    /// Measurement only. A quiet socket and a dead one look identical here,
    /// which is the whole reason heartbeats exist (§9).
    pub last_recv_ns: UnixNano,

    /// Socket-level errors absorbed on this socket.
    pub errors: u64,

    seen: Box<[bool]>,
}

impl SocketStats {
    /// Fresh counters for a socket configured against `filter`.
    pub fn new(endpoint: Endpoint, filter: &TrCodeFilter) -> Self {
        Self {
            endpoint,
            received: 0,
            published: 0,
            last_recv_ns: 0,
            errors: 0,
            seen: vec![false; filter.len()].into_boxed_slice(),
        }
    }

    /// Notes that the trcode at `index` in the allow-set arrived here.
    #[inline]
    pub fn mark_seen(&mut self, index: usize) {
        if let Some(slot) = self.seen.get_mut(index) {
            *slot = true;
        }
    }

    /// `true` if the trcode at `index` has ever arrived on this socket.
    #[inline]
    pub fn saw(&self, index: usize) -> bool {
        self.seen.get(index).copied().unwrap_or(false)
    }

    /// Codes configured for this feed that this socket has never carried.
    ///
    /// Non-empty is not automatically wrong — a feed's trcodes are spread over
    /// its sockets — but a code missing from *every* socket is a wiring fault or
    /// a market that never opened.
    pub fn never_seen<'a>(
        &'a self,
        filter: &'a TrCodeFilter,
    ) -> impl Iterator<Item = crate::trcode::TrCode> + 'a {
        filter
            .codes()
            .iter()
            .enumerate()
            .filter(move |(i, _)| !self.saw(*i))
            .map(|(_, c)| *c)
    }
}
