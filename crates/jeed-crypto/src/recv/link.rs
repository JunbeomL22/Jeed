//! The connection: TCP, TLS on top, WebSocket on top of that.
//!
//! ```text
//!  TcpStream ─(rustls)─→ bytes ──→ RecvBuffer ──peek──→ Frame ──consume──→ …
//!                                  (a frame boundary lands wherever it lands)
//! ```
//!
//! ## `std::net` plus `rustls`, and nothing else
//!
//! One TCP client needs `connect`, `set_nodelay`, `set_nonblocking`,
//! `set_read_timeout`, `read` and `write` — all in `std`, so there is no
//! `unsafe` and no `#[cfg(windows)]` here, and the tests run against a
//! loopback listener on any platform (`jeed_fix::recv::link` makes the same
//! argument). TLS is the one thing that cannot be written here, and
//! [`rustls::StreamOwned`] is a blocking `Read + Write` over a plain
//! `TcpStream`, so the loop is the same loop with or without it.
//!
//! ## The buffer is about boundaries, and about not copying
//!
//! TCP loses nothing; a `read` returns whatever arrived, which is half a
//! frame or four frames and a bit. A private `RecvBuffer` keeps the remainder and
//! [`Link::peek`] frames from its head each time. What it does **not** do is
//! copy a frame out: [`Frame::payload`] is a slice into the buffer, and the
//! decoder reads it there. That is only possible because a server never masks
//! (`super::ws`).
//!
//! Compaction happens once per [`fill`](Link::fill), not once per
//! [`consume`](Link::consume): a read that brought a hundred small frames
//! would otherwise move the tail a hundred times.

use crate::clock;
use crate::recv::endpoint::Endpoint;
use crate::recv::ws::{
    self, ACCEPT_LEN, KEY_LEN, Opcode, WsError, close_code, parse_header, parse_response,
    write_close, write_frame,
};
use core::fmt;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

/// Receive buffer, in bytes. Bounds one **frame**: a frame longer than this
/// can never complete and surfaces as [`LinkError::Oversize`].
///
/// A thousand-level Bybit snapshot is about fifty kilobytes and an OKX
/// four-hundred-level one about twenty; a megabyte is an order of magnitude
/// past the largest single frame any venue in this crate sends, and the cost
/// of being wrong is one reconnect with a clear error, not a silent loss.
pub const WS_RECV_BUFFER: usize = 1 << 20;

/// Reassembly buffer, in bytes. Bounds one **message** when the server
/// fragments it (`super::ws::assemble`).
pub const WS_MESSAGE_BUFFER: usize = 1 << 20;

/// Outbound buffer, in bytes. Bounds one frame we send — a subscription
/// naming a few hundred streams, or a pong.
pub const WS_SEND_BUFFER: usize = 16 * 1024;

/// Close code for an orderly shutdown (§7.4.1).
pub const CLOSE_NORMAL: u16 = 1000;

/// Close code for "going away" — the handler is stopping or giving up on a
/// silent peer (§7.4.1).
pub const CLOSE_GOING_AWAY: u16 = 1001;

/// Close code for a protocol error the peer committed (§7.4.1).
pub const CLOSE_PROTOCOL_ERROR: u16 = 1002;

/// How the connection is set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkOptions {
    /// `TCP_NODELAY`. On by default: a pong is six bytes and Nagle would hold
    /// it waiting for company that never comes.
    pub nodelay: bool,

    /// How long `connect` may take before it is called a failure.
    pub connect_timeout: Duration,

    /// How long the TLS and WebSocket handshakes together may take. The
    /// socket is blocking for this phase; the timeout is what bounds it.
    pub handshake_timeout: Duration,

    /// Blocking-mode read timeout. This is
    /// [`Mode::Block`](super::Mode)'s tick: a quiet connection wakes on it,
    /// and that wake-up is when pings and the liveness record happen.
    pub read_timeout: Duration,

    /// How long a `write` may block before it is called a failure.
    pub write_timeout: Duration,
}

impl Default for LinkOptions {
    fn default() -> Self {
        Self {
            nodelay: true,
            connect_timeout: Duration::from_secs(5),
            handshake_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_millis(100),
            write_timeout: Duration::from_secs(5),
        }
    }
}

/// The TLS client configuration, built once and shared by every connection.
///
/// Root certificates are Mozilla's bundle (`webpki-roots`) rather than the
/// OS store: one behaviour on Windows and Linux, and no second crate to talk
/// to `schannel`. A venue whose certificate does not chain to that bundle is
/// a venue whose certificate is wrong.
#[derive(Debug, Clone)]
pub struct Tls {
    config: Arc<ClientConfig>,
}

impl Tls {
    /// Mozilla roots, TLS 1.2 and 1.3, no client certificate.
    pub fn new() -> Self {
        let roots = RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        let config = ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("ring supports TLS 1.2 and 1.3")
            .with_root_certificates(roots)
            .with_no_client_auth();
        Self { config: Arc::new(config) }
    }

    /// A configuration built elsewhere — a private root, say.
    pub fn with_config(config: Arc<ClientConfig>) -> Self {
        Self { config }
    }
}

impl Default for Tls {
    fn default() -> Self {
        Self::new()
    }
}

/// One frame the server sent, borrowed from the receive buffer.
#[derive(Debug, Clone, Copy)]
pub struct Frame<'a> {
    /// `true` when this frame ends its message.
    pub fin: bool,

    /// What the frame is.
    pub opcode: Opcode,

    /// The payload, in place. A slice of the receive buffer.
    pub payload: &'a [u8],

    len: usize,
}

impl Frame<'_> {
    /// Bytes the whole frame occupies — what to [`consume`](Link::consume).
    #[inline]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// `true` for a frame with no payload.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.payload.is_empty()
    }

    /// The Close code, for a Close frame that carried one.
    #[inline]
    pub fn close_code(&self) -> Option<u16> {
        close_code(self.payload)
    }
}

/// Unread stream bytes, with a head that advances on consume and a
/// compaction that happens once per fill.
#[derive(Debug)]
struct RecvBuffer {
    buf: Box<[u8]>,
    head: usize,
    len: usize,
}

impl RecvBuffer {
    fn new(capacity: usize) -> Self {
        Self { buf: vec![0; capacity].into_boxed_slice(), head: 0, len: 0 }
    }

    #[inline]
    fn filled(&self) -> &[u8] {
        &self.buf[self.head..self.len]
    }

    #[inline]
    fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// Moves the unread tail to the front so the spare region is one piece.
    #[inline]
    fn compact(&mut self) {
        if self.head > 0 {
            self.buf.copy_within(self.head..self.len, 0);
            self.len -= self.head;
            self.head = 0;
        }
    }

    #[inline]
    fn spare(&mut self) -> &mut [u8] {
        &mut self.buf[self.len..]
    }

    #[inline]
    fn commit(&mut self, n: usize) {
        self.len = (self.len + n).min(self.buf.len());
    }

    #[inline]
    fn consume(&mut self, n: usize) {
        self.head = (self.head + n).min(self.len);
        if self.head == self.len {
            self.head = 0;
            self.len = 0;
        }
    }
}

/// Plain or TLS. The loop does not know which.
enum Stream {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Stream {
    fn tcp(&self) -> &TcpStream {
        match self {
            Self::Plain(s) => s,
            Self::Tls(s) => &s.sock,
        }
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(s) => s.read(buf),
            Self::Tls(s) => s.read(buf),
        }
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        match self {
            Self::Plain(s) => s.write_all(buf),
            Self::Tls(s) => s.write_all(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(s) => s.flush(),
            Self::Tls(s) => s.flush(),
        }
    }
}

impl fmt::Debug for Stream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plain(s) => f.debug_tuple("Plain").field(s).finish(),
            Self::Tls(s) => f.debug_tuple("Tls").field(&s.sock).finish(),
        }
    }
}

/// One open WebSocket connection: the stream, the bytes not yet framed, and
/// the buffer frames are masked into on the way out.
#[derive(Debug)]
pub struct Link {
    stream: Stream,
    peer: SocketAddr,
    buf: RecvBuffer,
    out: Box<[u8]>,
    mask: u64,
}

impl Link {
    /// Resolves, connects, negotiates TLS if the endpoint is `wss`, and
    /// completes the WebSocket handshake.
    ///
    /// Blocking throughout, bounded by [`LinkOptions::connect_timeout`] and
    /// [`LinkOptions::handshake_timeout`]. Resolution happens here rather
    /// than once at boot — see [`Endpoint`].
    pub fn connect(endpoint: &Endpoint, opts: LinkOptions, tls: &Tls) -> Result<Self, LinkError> {
        let peer = endpoint.resolve().map_err(|e| LinkError::io("resolve", e))?;
        let tcp = TcpStream::connect_timeout(&peer, opts.connect_timeout)
            .map_err(|e| LinkError::io("connect", e))?;
        Self::prepare(&tcp, opts)?;

        let stream = if endpoint.tls {
            let name = ServerName::try_from(endpoint.host.clone()).map_err(|_| LinkError::Name)?;
            let conn = ClientConnection::new(tls.config.clone(), name).map_err(LinkError::Tls)?;
            Stream::Tls(Box::new(StreamOwned::new(conn, tcp)))
        } else {
            Stream::Plain(tcp)
        };
        Self::open(stream, peer, endpoint, opts.handshake_timeout)
    }

    /// Completes the WebSocket handshake over an already-connected plain
    /// stream, ignoring `endpoint.tls`.
    ///
    /// For tests, and for a handler that obtained its socket elsewhere. The
    /// stream is what it is; the endpoint supplies only what the `GET` says.
    pub fn upgrade(tcp: TcpStream, endpoint: &Endpoint, opts: LinkOptions) -> Result<Self, LinkError> {
        let peer = tcp.peer_addr().map_err(|e| LinkError::io("peer_addr", e))?;
        Self::prepare(&tcp, opts)?;
        Self::open(Stream::Plain(tcp), peer, endpoint, opts.handshake_timeout)
    }

    fn prepare(tcp: &TcpStream, opts: LinkOptions) -> Result<(), LinkError> {
        tcp.set_nodelay(opts.nodelay).map_err(|e| LinkError::io("set_nodelay", e))?;
        tcp.set_write_timeout(Some(opts.write_timeout))
            .map_err(|e| LinkError::io("set_write_timeout", e))?;
        // Blocking with a timeout for the handshake; `set_mode` decides the
        // steady state afterwards.
        tcp.set_read_timeout(Some(opts.handshake_timeout))
            .map_err(|e| LinkError::io("set_read_timeout", e))
    }

    fn open(
        stream: Stream,
        peer: SocketAddr,
        endpoint: &Endpoint,
        timeout: Duration,
    ) -> Result<Self, LinkError> {
        let nonce = nonce()?;
        let mask = u64::from_le_bytes(nonce[..8].try_into().expect("8 bytes")) | 1;
        let mut link = Self {
            stream,
            peer,
            buf: RecvBuffer::new(WS_RECV_BUFFER),
            out: vec![0; WS_SEND_BUFFER].into_boxed_slice(),
            mask,
        };
        link.handshake(endpoint, &ws::key(&nonce), timeout)?;
        Ok(link)
    }

    /// Sends the upgrade request and reads the response, leaving whatever
    /// followed it — usually the first frame — in the buffer.
    fn handshake(
        &mut self,
        endpoint: &Endpoint,
        key: &[u8; KEY_LEN],
        timeout: Duration,
    ) -> Result<(), LinkError> {
        let expected: [u8; ACCEPT_LEN] = ws::accept(key);
        let n = ws::request(&mut self.out, endpoint, key)?;
        self.stream.write_all(&self.out[..n]).map_err(|e| LinkError::io("write", e))?;
        self.stream.flush().map_err(|e| LinkError::io("flush", e))?;

        let deadline = clock::now_ns().saturating_add(timeout.as_nanos() as u64);
        loop {
            if let Some(n) = parse_response(self.buf.filled(), &expected)? {
                self.buf.consume(n);
                return Ok(());
            }
            if clock::now_ns() > deadline {
                return Err(LinkError::Timeout("handshake"));
            }
            self.fill()?;
        }
    }

    /// Who we are connected to.
    #[inline]
    pub const fn peer(&self) -> SocketAddr {
        self.peer
    }

    /// Switches between busy-spin reads and parking in `read`.
    pub fn set_mode(&mut self, spin: bool, read_timeout: Duration) -> Result<(), LinkError> {
        let tcp = self.stream.tcp();
        tcp.set_nonblocking(spin).map_err(|e| LinkError::io("set_nonblocking", e))?;
        let timeout = if spin { None } else { Some(read_timeout) };
        tcp.set_read_timeout(timeout).map_err(|e| LinkError::io("set_read_timeout", e))
    }

    /// Reads once into the spare tail.
    ///
    /// `Ok(0)` means nothing was waiting — the ordinary answer on a spinning
    /// socket, and a timed-out wait on a parked one. A peer that closed the
    /// connection is [`LinkError::Closed`], not a zero-length read the caller
    /// might mistake for quiet.
    pub fn fill(&mut self) -> Result<usize, LinkError> {
        self.buf.compact();
        if self.buf.spare().is_empty() {
            // Nothing can be read until something is framed out of the way,
            // and nothing will frame, or `peek` would have said so.
            return Err(LinkError::Oversize { limit: self.buf.capacity() });
        }
        match self.stream.read(self.buf.spare()) {
            Ok(0) => Err(LinkError::Closed { code: None }),
            Ok(n) => {
                self.buf.commit(n);
                Ok(n)
            }
            Err(e) if would_block(&e) => Ok(0),
            // rustls reports a TCP close without `close_notify` this way. The
            // peer is gone either way.
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Err(LinkError::Closed { code: None }),
            Err(e) => Err(LinkError::io("read", e)),
        }
    }

    /// Frames the message at the head without consuming it.
    ///
    /// `Ok(None)` means more bytes are needed. An `Err` means the peer broke
    /// the protocol or sent a frame the buffer cannot hold; either way the
    /// connection goes.
    pub fn peek(&self) -> Result<Option<Frame<'_>>, LinkError> {
        let filled = self.buf.filled();
        let Some(h) = parse_header(filled)? else {
            return Ok(None);
        };
        let len = h.len();
        if len > self.buf.capacity() {
            return Err(LinkError::Oversize { limit: self.buf.capacity() });
        }
        if filled.len() < len {
            return Ok(None);
        }
        Ok(Some(Frame { fin: h.fin, opcode: h.opcode, payload: &filled[h.header_len..len], len }))
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

    /// Sends one masked frame.
    ///
    /// Short writes are looped over: half a frame on the wire is a
    /// desynchronised peer. On a spinning socket the final flush may report
    /// `WouldBlock` with bytes still queued in the TLS layer; they go out on
    /// the next read or write, so that is not a failure.
    pub fn send(&mut self, opcode: Opcode, payload: &[u8]) -> Result<(), LinkError> {
        let mask = self.next_mask();
        let n = write_frame(&mut self.out, opcode, payload, mask)?;
        self.write(n)
    }

    /// Sends a text frame — a subscription, or a venue's own ping.
    #[inline]
    pub fn send_text(&mut self, payload: &[u8]) -> Result<(), LinkError> {
        self.send(Opcode::Text, payload)
    }

    /// Sends a ping the peer must answer.
    #[inline]
    pub fn send_ping(&mut self, payload: &[u8]) -> Result<(), LinkError> {
        self.send(Opcode::Ping, payload)
    }

    /// Answers a ping, echoing its payload.
    #[inline]
    pub fn send_pong(&mut self, payload: &[u8]) -> Result<(), LinkError> {
        self.send(Opcode::Pong, payload)
    }

    /// Sends a Close with `code`.
    pub fn send_close(&mut self, code: u16) -> Result<(), LinkError> {
        let mask = self.next_mask();
        let n = write_close(&mut self.out, code, mask)?;
        self.write(n)
    }

    fn write(&mut self, n: usize) -> Result<(), LinkError> {
        self.stream.write_all(&self.out[..n]).map_err(|e| LinkError::io("write", e))?;
        match self.stream.flush() {
            Ok(()) => Ok(()),
            Err(e) if would_block(&e) => Ok(()),
            Err(e) => Err(LinkError::io("flush", e)),
        }
    }

    /// The next masking key. xorshift64 seeded from the handshake nonce:
    /// unpredictability is not required (`super::ws::frame::write_frame`),
    /// only that consecutive frames differ.
    #[inline]
    fn next_mask(&mut self) -> [u8; 4] {
        let mut x = self.mask;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.mask = x;
        (x as u32).to_le_bytes()
    }
}

/// Sixteen bytes from the TLS provider's random source, for the handshake
/// key and the mask seed.
fn nonce() -> Result<[u8; 16], LinkError> {
    let mut out = [0u8; 16];
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(&mut out)
        .map_err(|_| LinkError::Random)?;
    Ok(out)
}

/// `true` for the "nothing right now" answers a socket can give.
///
/// A non-blocking socket says `WouldBlock`; a blocking one with a read timeout
/// says `TimedOut` on Windows and `WouldBlock` on Unix. Both mean the same
/// thing to the loop.
#[inline]
fn would_block(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
    )
}

/// A connection failed, or the peer went away.
#[derive(Debug)]
pub enum LinkError {
    /// A socket call failed.
    Io {
        /// The step that failed, for locating it without a stack trace.
        call: &'static str,

        /// What the OS said.
        source: io::Error,
    },

    /// The peer closed the connection — with a Close frame carrying `code`,
    /// or by hanging up.
    Closed {
        /// The Close code, if the peer sent one.
        code: Option<u16>,
    },

    /// The peer broke the WebSocket protocol, or a frame we tried to send
    /// did not fit its buffer.
    Protocol(WsError),

    /// A frame longer than the receive buffer is on the stream.
    Oversize {
        /// Bytes the buffer holds.
        limit: usize,
    },

    /// A bounded step did not complete in time.
    Timeout(&'static str),

    /// The peer stopped answering pings.
    Silent,

    /// The host is not something a TLS certificate can be checked against.
    Name,

    /// TLS refused — a certificate that does not chain, a handshake that
    /// failed.
    Tls(rustls::Error),

    /// The random source could not produce a handshake nonce.
    Random,
}

impl LinkError {
    fn io(call: &'static str, source: io::Error) -> Self {
        Self::Io { call, source }
    }

    /// `true` when the peer hung up rather than something failing.
    ///
    /// Worth distinguishing: a venue closing at its daily restart is the
    /// connection ending normally, and logging it as an error would bury the
    /// one that is not.
    #[inline]
    pub const fn is_closed(&self) -> bool {
        matches!(self, Self::Closed { .. })
    }
}

impl From<WsError> for LinkError {
    #[inline]
    fn from(e: WsError) -> Self {
        Self::Protocol(e)
    }
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { call, source } => write!(f, "{call} failed: {source}"),
            Self::Closed { code: Some(code) } => write!(f, "the peer closed with code {code}"),
            Self::Closed { code: None } => write!(f, "the peer closed the connection"),
            Self::Protocol(e) => write!(f, "{e}"),
            Self::Oversize { limit } => {
                write!(f, "a frame longer than {limit} bytes is on the stream")
            }
            Self::Timeout(step) => write!(f, "{step} timed out"),
            Self::Silent => write!(f, "the peer stopped answering pings"),
            Self::Name => write!(f, "host is not a valid TLS server name"),
            Self::Tls(e) => write!(f, "TLS: {e}"),
            Self::Random => write!(f, "no random bytes for the handshake"),
        }
    }
}

impl std::error::Error for LinkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Protocol(e) => Some(e),
            Self::Tls(e) => Some(e),
            _ => None,
        }
    }
}
