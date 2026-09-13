//! 수신부 — one TCP session in, one ring out.
//!
//! ```text
//! conf ─┬─ peer     ──→ Endpoint ──→ Link      (connect; FrameBuffer reassembly)
//!       ├─ comp ids ──→ Emitter   ──┐          (Logon, Heartbeat, Resend, Logout)
//!       └─ symbols  ──→ SymbolFilter┴→ Pipeline ──→ MdAdapter → RecordSink → ring
//!                                          │
//!                       Receiver ──────────┘  connect / logon / silence / counters
//! ```
//!
//! ## This is not `jeed_krx::recv` with a different socket
//!
//! `documents/feed_handler.md` §10 puts the two feeds side by side, and every
//! row of that table lands somewhere in this module:
//!
//! | | KRX multicast | FIX session |
//! |---|---|---|
//! | 가입 | IGMP join, many ports | one `connect`, then a **Logon** |
//! | 프레이밍 | one datagram = one message | a byte stream, reassembled |
//! | 유실 | routine, healed by the next snapshot | does not happen — a gap is an incident |
//! | 순번 | 정보분배일련번호 is unusable (`CLAUDE.md`) | `MsgSeqNum` is the authority |
//! | 방향 | receive only | **bidirectional** — silence must be answered |
//!
//! The last row is the one that reshapes everything. A KRX handler that sends
//! nothing still receives; a FIX handler that sends nothing never receives a
//! byte, because the session begins with our Logon. That is why [`emit`](crate::recv::emit) exists
//! here and has no counterpart in `jeed-krx`, and why [`Pipeline::ingest`]
//! returns a [`Reply`] — what the protocol *owes* the counterparty is decided
//! in the transport-free stage and merely written by the loop.
//!
//! ## The seam is the same seam
//!
//! `jeed_krx::recv::pipeline` is "everything the receive loop does that has
//! nothing to do with sockets", and this [`pipeline`](crate::recv::pipeline) is the same stage for the
//! same reason: filter, decode, publish, heartbeat — testable without a
//! network, and publishing through the one [`RecordSink`](jeed_wire::RecordSink)
//! path so two handlers cannot grow two ways to the ring
//! (`documents/todo.md` §15).
//!
//! What is new is [`MdAdapter`]. KRX's pipeline can call `dispatch::decode` and
//! be done, because a 전문 already says which instrument it is about. A FIX
//! message does not: turning a `Symbol` into a `(venue, isin)` and an absent
//! `MDEntrySize` into a quantity is venue knowledge, and this crate has none
//! (`documents/feed_handler.md` §13). So the last step is a trait the venue
//! implements, and the only thing that changes when a second FIX venue arrives.
//!
//! ## Why there is no `unsafe` here
//!
//! `jeed_krx::recv::socket` declares nine Winsock entry points because
//! `IP_ADD_MEMBERSHIP` with an explicit interface, `SO_RCVBUF` read-back and
//! `WSAPoll` over many sockets are all things `std::net` will not do. None of
//! that applies to one TCP client: [`Link`] is a `std::net::TcpStream` with a
//! [`FrameBuffer`](crate::FrameBuffer) in front of it, so this module has no
//! `unsafe`, no `#[cfg(windows)]`, and its tests run against a loopback
//! listener on any platform.

/// Outbound session messages — the half `jeed-krx` never needs.
pub mod emit;
/// `host:port` — what the session connects to.
pub mod endpoint;
/// The symbol allow-set.
pub mod filter;
/// The TCP connection and its reassembly buffer.
pub mod link;
/// Frame → route → decode → adapt → publish, with no transport in it.
pub mod pipeline;
/// The connect / logon / read / answer loop.
pub mod receiver;
/// Counters the receive loop keeps.
pub mod stats;

pub use emit::{EmitError, Emitter, MAX_EMIT_LEN, SubscriptionRequest, SubscriptionType};
pub use endpoint::{Endpoint, EndpointError};
pub use filter::{SymbolFilter, parse_symbol_list};
pub use link::{FIX_RECV_BUFFER, Link, LinkError, LinkOptions};
pub use pipeline::{Ingested, MdAdapter, Outcome, Pipeline, Reply};
pub use receiver::{Config, LinkState, Mode, Receiver, SendError};
pub use stats::Stats;
