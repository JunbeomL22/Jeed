//! 수신부 — UDP multicast in, one ring out.
//!
//! ```text
//! conf ─┬─ sockets  ──→ Endpoint ──→ FeedSocket   (IGMP join; free filtering)
//!       ├─ trcodes  ──→ TrCodeFilter ─┐
//!       └─ isin     ──→ IsinFilter  ──┴→ Pipeline ──→ RecordSink → shm ring
//!                                          │
//!                       Receiver ──────────┘  spin / block, heartbeat, counters
//! ```
//!
//! ## The socket does not pick the decoder
//!
//! The first design assumed "one trcode = one multicast group = one socket" and
//! bound a decoder to each socket. The standard says otherwise: **one port
//! carries every 데이터구분 of a product group** — `B6` `G7` `A3` `V1` `Q2`
//! `A7` `O6` `R1` all mixed together — so the decoder is chosen at runtime from
//! `payload[0..5]` and the trcode list in `conf` is a real filter, not a
//! configuration cross-check (`documents/todo.md` §5).
//!
//! That is also why `sockets` and `trcodes` are two separate lists: where to
//! listen and what to keep are independent, and the mapping between them is a
//! circuit assignment nobody here can validate.
//!
//! ## Why this splits in two
//!
//! [`Pipeline`] is the part with no sockets in it — filter, decode, publish,
//! heartbeat. [`Receiver`] is the part that is all sockets — join, spin or
//! block, drain, tick. The seam is where `jeed-fix` will attach: a TCP session
//! frames its messages differently but publishes through the same stage, and
//! the path to the ring has to stay one path. It is also what makes the
//! interesting half testable without a network.

/// `(그룹IP, 포트)` — what a socket joins.
pub mod endpoint;
/// The trcode and 종목코드 allow-sets.
pub mod filter;
/// Filter → decode → publish, with no transport in it.
pub mod pipeline;
/// The socket loop.
#[cfg(windows)]
pub mod receiver;
/// Winsock multicast sockets and `WSAPoll`.
#[cfg(windows)]
pub mod socket;
/// Counters the receive loop keeps.
pub mod stats;

pub use endpoint::{Endpoint, EndpointError};
pub use filter::{IsinFilter, IsinListError, TrCodeFilter, parse_isin_list};
pub use pipeline::{Outcome, Pipeline};
pub use stats::{SocketStats, Stats};

#[cfg(windows)]
pub use receiver::{Config, MAX_DATAGRAM, Mode, Receiver};
#[cfg(windows)]
pub use socket::{FeedSocket, NetError, Poller, SocketOptions};
