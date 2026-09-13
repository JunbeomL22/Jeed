//! Multicast sockets and the readiness wait.
//!
//! | | Windows | Linux |
//! |---|---|---|
//! | API | Winsock (`ws2_32`) | BSD sockets (libc) |
//! | bind address | `INADDR_ANY` — **forced** | the group address |
//! | non-blocking | `ioctlsocket(FIONBIO)` | `fcntl(O_NONBLOCK)` |
//! | wait | `WSAPoll` | `poll` |
//! | oversized datagram | `WSAEMSGSIZE` | silently truncated |
//!
//! The entry points are declared inline on both rather than taken from a
//! bindings crate, for the same reason `jeed-shm` declares its own: a whole
//! platform surface is a lot to inherit for `socket`, `bind`, `setsockopt` and
//! friends, and this crate is linked by anything that wants a KRX decoder.
//!
//! ## ⚠️ What a bind filters is not the same on the two
//!
//! The Unix habit of binding to the **group address** — which makes the
//! destination address part of the demultiplexing, so one socket sees one
//! group — works on Linux and **does not work on Windows**: measured,
//! `bind(239.255.77.88:30883)` there returns `WSAEADDRNOTAVAIL` (10049), and a
//! bind to the local unicast address would not match a datagram addressed to a
//! group either. Windows multicast receivers bind to `INADDR_ANY`, so the bind
//! filters by **port alone** and a socket sees every group anyone on the host
//! has joined on that port.
//!
//! Each platform therefore does the right thing for itself, and
//! [`Receiver::new`](crate::recv::Receiver::new) applies the **stricter** of
//! the two rules everywhere: no two endpoints on one port. A conf that is valid
//! on Linux is then valid on Windows, which is worth more than the one
//! arrangement it forbids — KRX assigns a port per group anyway (10302 선물,
//! 10322 콜, 10323 풋).
//!
//! ## Errors that are not failures
//!
//! A receive loop that returns on the first odd errno stops the feed over one
//! packet. [`NetError::is_transient`] names the ones to count and step over —
//! an ICMP port-unreachable reported late (`WSAECONNRESET` / `ECONNREFUSED`), a
//! signalled wait (`WSAEINTR` / `EINTR`), and on Windows a datagram bigger than
//! the buffer (`WSAEMSGSIZE`). Linux has no counterpart for the last one: it
//! truncates without saying so, and the truncated message is then rejected by
//! the pipeline's length check instead.

#[cfg_attr(windows, path = "windows.rs")]
#[cfg_attr(unix, path = "posix.rs")]
mod imp;

use crate::recv::endpoint::Endpoint;
use core::fmt;

/// A failed call and the OS code it failed with.
type OsErr = (&'static str, i32);

/// How a socket is set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketOptions {
    /// `SO_RCVBUF`. The single most effective defence against dropping
    /// datagrams: the goal is not sub-µs latency but **not losing multicast
    /// packets** (`documents/feed_handler.md` §14), and the kernel buffer is
    /// what absorbs a scheduling hiccup in the receive loop.
    ///
    /// Linux doubles the request for bookkeeping and reports the doubled value
    /// back, and caps it at `net.core.rmem_max` unless the process has
    /// `CAP_NET_ADMIN` — which is why
    /// [`recv_buffer_bytes`](FeedSocket::recv_buffer_bytes) exists.
    pub recv_buffer_bytes: u32,

    /// Non-blocking, so `recv` returns [`None`] instead of parking the thread.
    /// Required for [`Mode::Spin`](crate::recv::Mode); harmless for
    /// [`Mode::Block`](crate::recv::Mode), which parks in the poll instead.
    pub nonblocking: bool,

    /// `SO_REUSEADDR` before the bind.
    ///
    /// Not needed to run — the endpoints are one per port — but it is what lets
    /// a capture or diagnostic tool listen on the same port while the handler
    /// runs, which is worth more than the sharing it permits costs.
    pub reuse_addr: bool,
}

impl Default for SocketOptions {
    fn default() -> Self {
        Self { recv_buffer_bytes: 8 << 20, nonblocking: true, reuse_addr: true }
    }
}

/// A joined multicast socket.
#[derive(Debug)]
pub struct FeedSocket {
    raw: imp::Raw,
    endpoint: Endpoint,
}

// The handle is owned exclusively and every call through it takes `&self` or
// `&mut self`, so moving one onto its pinned receive thread is sound.
unsafe impl Send for FeedSocket {}

impl FeedSocket {
    /// Creates the socket, binds it, and joins the group.
    pub fn join(endpoint: Endpoint, opts: SocketOptions) -> Result<Self, NetError> {
        if !endpoint.group_is_multicast() {
            return Err(NetError { call: "join", endpoint, code: 0 });
        }
        let raw = imp::join(endpoint, opts).map_err(|e| NetError::from_os(e, endpoint))?;
        Ok(Self { raw, endpoint })
    }

    /// The endpoint this socket joined.
    #[inline]
    pub const fn endpoint(&self) -> Endpoint {
        self.endpoint
    }

    /// Reads one datagram.
    ///
    /// `Ok(None)` means the socket is non-blocking and empty — the normal
    /// answer on a spinning loop, and not an error.
    #[inline]
    pub fn recv(&self, buf: &mut [u8]) -> Result<Option<usize>, NetError> {
        imp::recv(self.raw, buf).map_err(|e| NetError::from_os(e, self.endpoint))
    }

    /// Switches the socket between blocking and non-blocking.
    pub fn set_nonblocking(&self, on: bool) -> Result<(), NetError> {
        imp::set_nonblocking(self.raw, on).map_err(|e| NetError::from_os(e, self.endpoint))
    }

    /// The receive buffer the kernel actually granted.
    ///
    /// Worth reading back: `SO_RCVBUF` is a request, and a refused or capped
    /// one is the difference between absorbing a scheduling hiccup and dropping
    /// a burst.
    pub fn recv_buffer_bytes(&self) -> Result<u32, NetError> {
        imp::recv_buffer_bytes(self.raw).map_err(|e| NetError::from_os(e, self.endpoint))
    }
}

impl Drop for FeedSocket {
    fn drop(&mut self) {
        // Closing the socket drops the group membership with it.
        imp::close(self.raw);
    }
}

/// Readiness wait over a fixed set of sockets.
///
/// Used by [`Mode::Block`](crate::recv::Mode) only. Its timeout **is** the
/// loop's tick: a cold feed that receives nothing still wakes up once per
/// timeout, which is when the heartbeat gets written
/// (`documents/todo.md` §5).
#[derive(Debug)]
pub struct Poller {
    inner: imp::Poll,
}

impl Poller {
    /// Watches these sockets for readable data, in the order given.
    pub fn new(sockets: &[FeedSocket]) -> Self {
        Self { inner: imp::Poll::new(sockets.iter().map(|s| s.raw)) }
    }

    /// Waits up to `timeout_ms` and returns how many sockets are readable.
    ///
    /// With no sockets it returns `Ok(0)` without calling the OS, which on
    /// Windows rejects an empty set — a feed configured with no sockets should
    /// idle, not fail on the first tick.
    pub fn wait(&mut self, timeout_ms: i32) -> Result<usize, NetError> {
        self.inner.wait(timeout_ms).map_err(|e| {
            NetError::from_os(e, Endpoint::new(std::net::Ipv4Addr::UNSPECIFIED, 0))
        })
    }

    /// `true` if the socket at `index` had data on the last
    /// [`wait`](Self::wait).
    #[inline]
    pub fn is_ready(&self, index: usize) -> bool {
        self.inner.is_ready(index)
    }

    /// Number of sockets watched.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// `true` when no socket is watched.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.len() == 0
    }
}

/// A socket call failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetError {
    /// The step that failed — an OS call, or one of our own checks, for
    /// locating it without a stack trace. Platform-specific, deliberately: the
    /// useful thing to do with this is search for it.
    pub call: &'static str,

    /// The endpoint being set up or read.
    pub endpoint: Endpoint,

    /// `WSAGetLastError()` on Windows, `errno` on POSIX. Zero when the failure
    /// was our own check rather than the OS's.
    pub code: i32,
}

impl NetError {
    fn from_os((call, code): OsErr, endpoint: Endpoint) -> Self {
        Self { call, endpoint, code }
    }

    /// `true` for errors a receive loop should count and step over.
    ///
    /// See the module docs: a late ICMP port-unreachable, a signalled wait, and
    /// (Windows only) one oversized datagram. None of them means the socket is
    /// unusable, and stopping the feed on any of them would trade the whole
    /// market for one packet.
    #[inline]
    pub fn is_transient(&self) -> bool {
        imp::is_transient(self.code)
    }
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} failed on {}", self.call, self.endpoint)?;
        if self.code != 0 {
            let source = if cfg!(windows) { "WSA" } else { "errno" };
            write!(f, " ({source} {})", self.code)?;
        }
        Ok(())
    }
}

impl std::error::Error for NetError {}
