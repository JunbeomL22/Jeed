//! RFC 6455, the client half, without extensions.
//!
//! ```text
//!  handshake   GET … Upgrade: websocket ──→ 101 + Sec-WebSocket-Accept   (once)
//!  frame       [FIN|op][len][payload]     ←── server, never masked
//!              [FIN|op][len][mask][payload] ──→ us, always masked
//!  assemble    fragment + continuation… + FIN → one message
//! ```
//!
//! ## What is here and what is not
//!
//! Enough of the protocol to hold a market-data subscription open for a
//! trading day: the opening handshake and its proof-of-protocol check, both
//! directions of framing, fragmentation, and the three control frames. What
//! is deliberately absent:
//!
//! - **Extensions.** We offer none, so `permessage-deflate` is never
//!   negotiated and a frame with a reserved bit set is a protocol error
//!   rather than compressed data. A venue that compresses unconditionally
//!   (HTX) is outside this crate for that reason (`documents/todo.md`).
//! - **Subprotocols.** No venue asks for one.
//! - **A server.** Nothing here accepts a connection.
//!
//! ## Why the receive path copies nothing
//!
//! A server **must not** mask the frames it sends (RFC 6455 §5.1), so a
//! payload arrives in the receive buffer exactly as the decoder wants to read
//! it — [`Frame::payload`](crate::recv::Frame::payload) is a slice into that
//! buffer, not a copy out of it. Masking is client-to-server only, and our
//! traffic in that direction is a subscription at connect and a pong every
//! few seconds. That asymmetry is what makes writing this cheaper than
//! adopting a library whose `read()` hands back an owned `String` per frame.
//!
//! The one copy is [`assemble`](crate::recv::ws::assemble): a message the server chose to fragment is
//! reassembled into its own buffer, because the decoders walk one contiguous
//! slice. Most venues never fragment; a large snapshot on some will.

pub mod assemble;
pub mod frame;
pub mod handshake;
pub mod sha1;

use core::fmt;

pub use assemble::Assembly;
pub use frame::{
    FrameHeader, MAX_CONTROL_PAYLOAD, MAX_HEADER_LEN, Opcode, close_code, framed_len, parse_header,
    write_close, write_frame,
};
pub use handshake::{ACCEPT_LEN, KEY_LEN, accept, key, parse_response, request};

/// The peer, or our own outbound buffer, broke the protocol.
///
/// Every variant but [`NoRoom`](Self::NoRoom) is fatal to the connection: a
/// WebSocket stream that has gone wrong at one frame boundary offers nothing
/// to resynchronise on, and RFC 6455 says to fail the connection. The
/// receive loop does exactly that and reconnects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsError {
    /// A reserved bit (`RSV1..3`) was set. We negotiated no extension, so
    /// there is nothing it could mean.
    ReservedBits,

    /// An opcode this build does not know.
    Opcode(u8),

    /// The server masked a frame. Servers must not (§5.1), and a masked
    /// payload is not the bytes the decoder expects.
    MaskedByServer,

    /// A 64-bit length with its high bit set, which the protocol forbids.
    Length,

    /// A control frame with `FIN` clear. Control frames may not be
    /// fragmented (§5.5).
    FragmentedControl,

    /// A control frame longer than [`MAX_CONTROL_PAYLOAD`].
    ControlTooLong(usize),

    /// A continuation frame arrived with no message open.
    UnexpectedContinuation,

    /// A new text or binary frame arrived while a fragmented message was
    /// still open. Data frames may not interleave (§5.4).
    InterleavedMessage,

    /// A fragmented message outgrew the reassembly buffer.
    MessageTooLong {
        /// Bytes the buffer holds.
        limit: usize,
    },

    /// An outbound frame does not fit the buffer it was to be written to.
    NoRoom,

    /// The handshake response was not `101 Switching Protocols`.
    Status(u16),

    /// The response's status line or a header line could not be read.
    MalformedResponse,

    /// No `Upgrade: websocket`.
    Upgrade,

    /// No `Connection: Upgrade`.
    Connection,

    /// `Sec-WebSocket-Accept` absent or not the hash of our key. Whatever
    /// answered is not a WebSocket server, or not the one we asked.
    Accept,

    /// The server named an extension we did not offer.
    Extension,

    /// The server named a subprotocol we did not offer.
    Subprotocol,
}

impl fmt::Display for WsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReservedBits => write!(f, "reserved bit set with no extension negotiated"),
            Self::Opcode(op) => write!(f, "unknown opcode {op:#x}"),
            Self::MaskedByServer => write!(f, "the server masked a frame"),
            Self::Length => write!(f, "64-bit payload length with the high bit set"),
            Self::FragmentedControl => write!(f, "control frame without FIN"),
            Self::ControlTooLong(n) => {
                write!(f, "control frame payload of {n} bytes exceeds {MAX_CONTROL_PAYLOAD}")
            }
            Self::UnexpectedContinuation => write!(f, "continuation frame with no message open"),
            Self::InterleavedMessage => write!(f, "data frame while a fragmented message is open"),
            Self::MessageTooLong { limit } => {
                write!(f, "fragmented message exceeds the {limit}-byte reassembly buffer")
            }
            Self::NoRoom => write!(f, "outbound frame does not fit its buffer"),
            Self::Status(code) => write!(f, "handshake answered with HTTP {code}, not 101"),
            Self::MalformedResponse => write!(f, "handshake response could not be read"),
            Self::Upgrade => write!(f, "handshake response lacks `Upgrade: websocket`"),
            Self::Connection => write!(f, "handshake response lacks `Connection: Upgrade`"),
            Self::Accept => write!(f, "`Sec-WebSocket-Accept` is missing or wrong"),
            Self::Extension => write!(f, "the server selected an extension we did not offer"),
            Self::Subprotocol => write!(f, "the server selected a subprotocol we did not offer"),
        }
    }
}

impl core::error::Error for WsError {}
