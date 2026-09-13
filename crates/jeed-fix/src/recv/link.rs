//! The TCP connection and its reassembly buffer.
//!
//! ```text
//!  TcpStream ──read──→ FrameBuffer<N> ──peek──→ Frame ──consume──→ …
//!                       (a message boundary lands wherever it lands)
//! ```
//!
//! ## `std::net`, where `jeed-krx` declares Winsock by hand
//!
//! That crate declares nine `ws2_32` entry points because nothing in `std` will
//! do `IP_ADD_MEMBERSHIP` with a chosen interface, read `SO_RCVBUF` back, or
//! poll many sockets at once — and all three decide whether a multicast feed
//! loses packets (`documents/feed_handler.md` §14).
//!
//! None of that applies to one TCP client. `connect`, `set_nodelay`,
//! `set_nonblocking`, `set_read_timeout` and `read` are all in `std`, so this
//! module has no `unsafe`, no `#[cfg(windows)]`, and its tests run against a
//! loopback listener anywhere. Declaring Winsock here would buy nothing and
//! cost the crate its portability and its test.
//!
//! ## TCP loses nothing, so the buffer is about boundaries
//!
//! A UDP receive buffer exists so a scheduling hiccup does not drop datagrams.
//! This one exists because a stream has no boundaries: a `read` returns
//! whatever arrived, which is half a snapshot, or three messages and a bit. So
//! [`FrameBuffer`](crate::FrameBuffer) keeps the remainder and
//! [`peek`](crate::FrameBuffer::peek) re-frames from the start each time.
//! Nothing is lost by a short read — it is simply not a message yet.

use crate::error::FixError;
use crate::frame::{Frame, FrameBuffer};
use crate::recv::endpoint::Endpoint;
use core::fmt;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// Reassembly buffer for one session, in bytes.
///
/// A 10-deep two-sided snapshot with per-entry timestamps is under 1 KiB and
/// [`MD_MAX_ENTRIES`](crate::MD_MAX_ENTRIES) caps a message at 32 entries, so
/// this holds any message the decoder can produce several times over. It is not
/// a socket buffer: TCP's own window absorbs bursts, and what is left here
/// between reads is one partial message.
pub const FIX_RECV_BUFFER: usize = 16 * 1024;

/// How the connection is set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkOptions {
    /// `TCP_NODELAY`. On by default and it should stay on: Nagle would hold
    /// our 60-byte Heartbeat waiting for company that never comes, and the
    /// venue would time the session out while we waited.
    pub nodelay: bool,

    /// How long `connect` may take before it is called a failure.
    pub connect_timeout: Duration,

    /// Blocking-mode read timeout. This is
    /// [`Mode::Block`](super::Mode)'s tick: a session with nothing on it wakes
    /// on this, and that wake-up is when heartbeats and silence checks happen.
    pub read_timeout: Duration,

    /// How long a `write` may block before it is called a failure. A venue that
    /// stops reading would otherwise park the receive thread inside a Heartbeat.
    pub write_timeout: Duration,
}

impl Default for LinkOptions {
    fn default() -> Self {
        Self {
            nodelay: true,
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_millis(100),
            write_timeout: Duration::from_secs(5),
        }
    }
}

/// One connected session: the socket, and the bytes not yet framed.
#[derive(Debug)]
pub struct Link {
    stream: TcpStream,
    peer: SocketAddr,
    buf: Box<FrameBuffer<FIX_RECV_BUFFER>>,
}

impl Link {
    /// Resolves, connects, and applies `opts`.
    ///
    /// Resolution happens here rather than once at boot — see
    /// [`Endpoint`].
    pub fn connect(endpoint: &Endpoint, opts: LinkOptions) -> Result<Self, LinkError> {
        let peer = endpoint.resolve().map_err(|e| LinkError::new("resolve", e))?;
        let stream = TcpStream::connect_timeout(&peer, opts.connect_timeout)
            .map_err(|e| LinkError::new("connect", e))?;
        stream.set_nodelay(opts.nodelay).map_err(|e| LinkError::new("set_nodelay", e))?;
        stream
            .set_write_timeout(Some(opts.write_timeout))
            .map_err(|e| LinkError::new("set_write_timeout", e))?;
        Ok(Self { stream, peer, buf: Box::new(FrameBuffer::new()) })
    }

    /// Wraps an already-connected stream. For tests, and for a handler that
    /// obtained its socket elsewhere.
    pub fn from_stream(stream: TcpStream) -> Result<Self, LinkError> {
        let peer = stream.peer_addr().map_err(|e| LinkError::new("peer_addr", e))?;
        Ok(Self { stream, peer, buf: Box::new(FrameBuffer::new()) })
    }

    /// Who we are connected to.
    #[inline]
    pub const fn peer(&self) -> SocketAddr {
        self.peer
    }

    /// Switches between busy-spin reads and parking in `read`.
    pub fn set_mode(&mut self, spin: bool, read_timeout: Duration) -> Result<(), LinkError> {
        self.stream.set_nonblocking(spin).map_err(|e| LinkError::new("set_nonblocking", e))?;
        let timeout = if spin { None } else { Some(read_timeout) };
        self.stream.set_read_timeout(timeout).map_err(|e| LinkError::new("set_read_timeout", e))
    }

    /// Reads once into the spare tail.
    ///
    /// `Ok(0)` means nothing was waiting — the ordinary answer on a spinning
    /// socket, and a timed-out wait on a parked one. A peer that closed the
    /// connection is a [`LinkError`] that [`is_closed`](LinkError::is_closed),
    /// not a zero-length read the caller might mistake for quiet.
    pub fn fill(&mut self) -> Result<usize, LinkError> {
        if self.buf.spare().is_empty() {
            // Nothing can be read until something is framed out of the way, and
            // nothing will frame, or `peek` would have said so. The stream is
            // carrying a message longer than the buffer can hold.
            return Err(LinkError::oversize());
        }
        match self.stream.read(self.buf.spare()) {
            Ok(0) => Err(LinkError::closed()),
            Ok(n) => {
                self.buf.commit(n);
                Ok(n)
            }
            Err(e) if would_block(&e) => Ok(0),
            Err(e) => Err(LinkError::new("read", e)),
        }
    }

    /// Frames the message at the head without consuming it.
    ///
    /// `Ok(None)` means more bytes are needed. An `Err` means the stream is out
    /// of sync at the current byte, and there is no resynchronising from that:
    /// a `BodyLength` that lied means every boundary after it is a guess
    /// ([`frame`](crate::frame())). The connection goes.
    #[inline]
    pub fn peek(&self) -> Result<Option<Frame<'_>>, FixError> {
        self.buf.peek()
    }

    /// Drops the first `n` bytes, `n` being a framed message's length.
    #[inline]
    pub fn consume(&mut self, n: usize) {
        self.buf.consume(n);
    }

    /// Bytes held that have not framed into a message yet.
    #[inline]
    pub fn pending(&self) -> usize {
        self.buf.filled().len()
    }

    /// Writes one complete message.
    ///
    /// Short writes are looped over: a partial FIX message on the wire is a
    /// desynchronised session at the other end, and the venue would see the
    /// tail of our Logon as the head of the next message.
    pub fn send(&mut self, bytes: &[u8]) -> Result<(), LinkError> {
        self.stream.write_all(bytes).map_err(|e| LinkError::new("write", e))?;
        self.stream.flush().map_err(|e| LinkError::new("flush", e))
    }
}

/// `true` for the two "nothing right now" answers a read can give.
///
/// A non-blocking socket says `WouldBlock`; a blocking one with a read timeout
/// says `TimedOut` on Windows and `WouldBlock` on Unix. Both mean the same
/// thing to the loop, and treating either as an error would tear down a session
/// on its first quiet tick.
#[inline]
fn would_block(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)
        || e.kind() == io::ErrorKind::Interrupted
}

/// A socket operation failed, or the peer went away.
#[derive(Debug)]
pub struct LinkError {
    /// The step that failed, for locating it without a stack trace.
    pub call: &'static str,

    /// The underlying error, if there was one. Absent for our own checks.
    pub source: Option<io::Error>,
}

impl LinkError {
    fn new(call: &'static str, source: io::Error) -> Self {
        Self { call, source: Some(source) }
    }

    /// The peer closed the connection.
    fn closed() -> Self {
        Self { call: "closed", source: None }
    }

    /// A message longer than the reassembly buffer can hold.
    fn oversize() -> Self {
        Self { call: "oversize", source: None }
    }

    /// `true` when the peer hung up rather than something failing.
    ///
    /// Worth distinguishing: an orderly close after a Logout is the session
    /// ending normally, and logging it as an error would bury the one that is
    /// not.
    #[inline]
    pub fn is_closed(&self) -> bool {
        self.call == "closed"
    }
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.source, self.call) {
            (_, "closed") => write!(f, "the peer closed the connection"),
            (_, "oversize") => {
                write!(f, "a message longer than {FIX_RECV_BUFFER} bytes is on the stream")
            }
            (Some(e), call) => write!(f, "{call} failed: {e}"),
            (None, call) => write!(f, "{call} failed"),
        }
    }
}

impl std::error::Error for LinkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|e| e as &(dyn std::error::Error + 'static))
    }
}
