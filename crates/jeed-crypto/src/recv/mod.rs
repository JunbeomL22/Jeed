//! 수신부 — one WebSocket in, one ring out.
//!
//! ```text
//! conf ─┬─ url    ──→ Endpoint ──→ Link     (TCP · rustls · GET/101 · frames)
//!       └─ router ──→ Pipeline ──→ Router ──→ RecordSink → ring
//!                          │
//!          Receiver ───────┘  connect / ping / keepalive / reconnect / counters
//! ```
//!
//! ## The third receive loop, and why it is not the second one reused
//!
//! `jeed_krx::recv` joins multicast groups and never sends. `jeed_fix::recv`
//! holds one TCP conversation and must log on before it hears a byte. This
//! one is TCP too, and shares the FIX loop's shape — connect, answer
//! silence, reconnect, one pinned core, one ring — but three things are its
//! own:
//!
//! | | FIX | WebSocket |
//! |---|---|---|
//! | 세션 시작 | `35=A`, answered later | `GET` … `101`, done before the loop sees it |
//! | 프레이밍 | `8=FIX…10=xxx` in a byte stream | RFC 6455 frames, **possibly fragmented** |
//! | 생존 | Heartbeat / TestRequest, one layer | protocol ping **and** a venue ping in its own dialect |
//!
//! ## The transport is the only new dependency
//!
//! `wss://` is TLS, and TLS is `rustls` — the one crate this workspace takes
//! from outside, behind the `recv` feature. WebSocket framing is written here
//! ([`ws`]) rather than adopted, because a server never masks and the payload
//! can therefore be decoded **in the receive buffer**; a library that returns
//! an owned message per frame would put an allocation on the one path this
//! crate keeps clean. The trade is a few hundred lines of RFC 6455, tested
//! against a loopback server that speaks it.
//!
//! ## The seam is the same seam
//!
//! [`Pipeline`] is "everything the receive loop does that has nothing to do
//! with sockets" — the same stage as the other two handlers, publishing
//! through the one [`RecordSink`](jeed_wire::RecordSink) path so three
//! handlers cannot grow three ways to the ring (`documents/todo.md` §15).
//! [`Router`] is where the venue attaches: which stream a message belongs
//! to, and what ping the venue wants to hear, are the two things that change
//! per venue and the loop does not know either.

/// `wss://host:port/path` — what the loop connects to.
pub mod endpoint;
/// The connection: TCP, TLS, and WebSocket frames in and out.
pub mod link;
/// Route → decode → publish, with no transport in it.
pub mod pipeline;
/// The connect / read / answer loop.
pub mod receiver;
/// Counters the receive loop keeps.
pub mod stats;
/// RFC 6455 — handshake, frames, fragments.
pub mod ws;

pub use endpoint::{Endpoint, EndpointError};
pub use link::{
    CLOSE_GOING_AWAY, CLOSE_NORMAL, CLOSE_PROTOCOL_ERROR, Frame, Link, LinkError, LinkOptions, Tls,
    WS_MESSAGE_BUFFER, WS_RECV_BUFFER, WS_SEND_BUFFER,
};
pub use pipeline::{MAX_KEEPALIVE_LEN, Outcome, Pipeline, Router};
pub use receiver::{Config, LinkState, Mode, Receiver, SendError};
pub use stats::Stats;
pub use ws::{Opcode, WsError};
