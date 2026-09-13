//! Winsock: joining a multicast group and reading datagrams off it.
//!
//! The nine entry points below are declared inline rather than taken from a
//! bindings crate, for the same reason `jeed-shm` declares its six
//! (`jeed-shm/Cargo.toml`): a whole Win32 surface is a lot to inherit for
//! `socket`, `bind`, `setsockopt` and friends, and this crate is linked by
//! anything that wants a KRX decoder.
//!
//! ## The bind cannot filter, on Windows
//!
//! The Unix habit of binding to the group address — which makes the
//! destination address part of the demultiplexing — **does not work here**:
//! measured, `bind(239.255.77.88:30883)` returns `WSAEADDRNOTAVAIL` (10049).
//! Windows binds multicast receivers to `INADDR_ANY` and nothing else; a bind
//! to the local unicast address would not match a datagram addressed to a
//! group either.
//!
//! The consequence is that **the bind filters by port alone**. A socket bound
//! to `0.0.0.0:port` sees every group any socket on the host has joined on that
//! port, so two endpoints sharing a port would each receive both streams and
//! every message would be decoded and published twice. That is why
//! [`Receiver::new`](crate::recv::Receiver::new) refuses a configuration with
//! two endpoints on one port, rather than leaving it to be discovered as a
//! doubled book.
//!
//! ## Two errors that are not failures
//!
//! `WSAECONNRESET` arrives on a UDP socket when an earlier send drew an ICMP
//! port-unreachable; `WSAEMSGSIZE` means a datagram was larger than the buffer
//! and the tail is gone. Neither says the socket is broken, and a receive loop
//! that returns on them stops the feed over one bad packet. They are counted
//! and skipped — see [`NetError::is_transient`].

use crate::recv::endpoint::Endpoint;
use core::ffi::c_void;
use core::fmt;
use std::net::Ipv4Addr;
use std::sync::Once;

/// Windows `SOCKET` — a `UINT_PTR`, not a file descriptor.
type RawSocket = usize;

const INVALID_SOCKET: RawSocket = RawSocket::MAX;
const SOCKET_ERROR: i32 = -1;

const AF_INET: i32 = 2;
const SOCK_DGRAM: i32 = 2;
const IPPROTO_UDP: i32 = 17;
const IPPROTO_IP: i32 = 0;
const SOL_SOCKET: i32 = 0xffff;
const SO_REUSEADDR: i32 = 0x0004;
const SO_RCVBUF: i32 = 0x1002;
const IP_ADD_MEMBERSHIP: i32 = 12;
const FIONBIO: i32 = 0x8004_667e_u32 as i32;
const POLLRDNORM: i16 = 0x0100;

/// Winsock error codes this crate names.
const WSAEINTR: i32 = 10004;
const WSAEMSGSIZE: i32 = 10040;
const WSAEWOULDBLOCK: i32 = 10035;
const WSAECONNRESET: i32 = 10054;

#[repr(C)]
struct SockAddrIn {
    sin_family: u16,
    sin_port: u16,
    sin_addr: u32,
    sin_zero: [u8; 8],
}

#[repr(C)]
struct IpMreq {
    imr_multiaddr: u32,
    imr_interface: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct WsaPollFd {
    fd: RawSocket,
    events: i16,
    revents: i16,
}

const _: () = assert!(size_of::<SockAddrIn>() == 16);
const _: () = assert!(size_of::<IpMreq>() == 8);
#[cfg(target_arch = "x86_64")]
const _: () = assert!(size_of::<WsaPollFd>() == 16);

/// `WSADATA` is only ever written, never read, so it is a correctly sized and
/// aligned block of bytes rather than a transcribed struct whose layout differs
/// between 32- and 64-bit Windows.
#[repr(C, align(8))]
struct WsaData([u8; 512]);

#[link(name = "ws2_32")]
unsafe extern "system" {
    fn WSAStartup(version: u16, data: *mut WsaData) -> i32;
    fn WSAGetLastError() -> i32;
    fn socket(af: i32, ty: i32, protocol: i32) -> RawSocket;
    fn bind(s: RawSocket, name: *const SockAddrIn, namelen: i32) -> i32;
    fn setsockopt(s: RawSocket, level: i32, optname: i32, optval: *const c_void, optlen: i32)
    -> i32;
    fn getsockopt(
        s: RawSocket,
        level: i32,
        optname: i32,
        optval: *mut c_void,
        optlen: *mut i32,
    ) -> i32;
    fn ioctlsocket(s: RawSocket, cmd: i32, argp: *mut u32) -> i32;
    fn recv(s: RawSocket, buf: *mut u8, len: i32, flags: i32) -> i32;
    fn closesocket(s: RawSocket) -> i32;
    fn WSAPoll(fdarray: *mut WsaPollFd, nfds: u32, timeout: i32) -> i32;
}

/// Initialises Winsock once per process.
///
/// `WSACleanup` is deliberately never called: the matching teardown would have
/// to run after every socket is closed, and this process lives as long as the
/// trading day.
fn winsock_init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let mut data = WsaData([0; 512]);
        // SAFETY: `data` is a live, correctly aligned block at least as large
        // as `WSADATA`, which is the only thing WSAStartup writes.
        let rc = unsafe { WSAStartup(0x0202, &raw mut data) };
        debug_assert_eq!(rc, 0, "WSAStartup failed");
    });
}

/// How a socket is set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketOptions {
    /// `SO_RCVBUF`. The single most effective defence against dropping
    /// datagrams: the goal on Windows is not sub-µs latency but **not losing
    /// multicast packets** (`documents/feed_handler.md` §14), and the kernel
    /// buffer is what absorbs a scheduling hiccup in the receive loop.
    pub recv_buffer_bytes: u32,

    /// Non-blocking, so `recv` returns [`None`] instead of parking the thread.
    /// Required for [`Mode::Spin`](crate::recv::Mode); harmless for
    /// [`Mode::Block`](crate::recv::Mode), which parks in `WSAPoll` instead.
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
    raw: RawSocket,
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
        winsock_init();

        // SAFETY: a plain socket creation; the arguments are the constants
        // Winsock defines for a UDP/IPv4 datagram socket.
        let raw = unsafe { socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP) };
        if raw == INVALID_SOCKET {
            return Err(NetError::last("socket", endpoint));
        }
        let sock = Self { raw, endpoint };

        if opts.reuse_addr {
            sock.set_opt(SOL_SOCKET, SO_REUSEADDR, &1i32, "SO_REUSEADDR")?;
        }
        sock.set_opt(
            SOL_SOCKET,
            SO_RCVBUF,
            &(opts.recv_buffer_bytes as i32),
            "SO_RCVBUF",
        )?;

        // `INADDR_ANY` is the only address Windows accepts here — see the
        // module docs. The group is selected by the membership below, and the
        // port is all the bind contributes.
        let addr = SockAddrIn {
            sin_family: AF_INET as u16,
            sin_port: endpoint.port.to_be(),
            sin_addr: u32::from_ne_bytes(Ipv4Addr::UNSPECIFIED.octets()),
            sin_zero: [0; 8],
        };
        // SAFETY: `addr` is a live, fully initialised `sockaddr_in` and the
        // length matches its size.
        if unsafe { bind(sock.raw, &raw const addr, size_of::<SockAddrIn>() as i32) }
            == SOCKET_ERROR
        {
            return Err(NetError::last("bind", endpoint));
        }

        // The membership must follow the bind on Windows.
        let mreq = IpMreq {
            imr_multiaddr: u32::from_ne_bytes(endpoint.group.octets()),
            imr_interface: u32::from_ne_bytes(endpoint.interface.octets()),
        };
        sock.set_opt(IPPROTO_IP, IP_ADD_MEMBERSHIP, &mreq, "IP_ADD_MEMBERSHIP")?;

        if opts.nonblocking {
            sock.set_nonblocking(true)?;
        }
        Ok(sock)
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
        let len = buf.len().min(i32::MAX as usize) as i32;
        // SAFETY: `buf` is a live, writable slice of at least `len` bytes, and
        // `recv` writes no more than that.
        let n = unsafe { recv(self.raw, buf.as_mut_ptr(), len, 0) };
        if n >= 0 {
            return Ok(Some(n as usize));
        }
        let e = NetError::last("recv", self.endpoint);
        if e.code == WSAEWOULDBLOCK { Ok(None) } else { Err(e) }
    }

    /// Switches the socket between blocking and non-blocking.
    pub fn set_nonblocking(&self, on: bool) -> Result<(), NetError> {
        let mut flag: u32 = u32::from(on);
        // SAFETY: `FIONBIO` takes a pointer to one `u_long`, which is what
        // `flag` is.
        if unsafe { ioctlsocket(self.raw, FIONBIO, &raw mut flag) } == SOCKET_ERROR {
            return Err(NetError::last("ioctlsocket", self.endpoint));
        }
        Ok(())
    }

    /// The receive buffer the kernel actually granted.
    ///
    /// Worth reading back: `SO_RCVBUF` is a request, and a refused one is the
    /// difference between absorbing a scheduling hiccup and dropping a burst.
    pub fn recv_buffer_bytes(&self) -> Result<u32, NetError> {
        let mut value: i32 = 0;
        let mut len = size_of::<i32>() as i32;
        // SAFETY: `value` and `len` are live; `len` states the buffer size and
        // `getsockopt` writes no more than that.
        let rc = unsafe {
            getsockopt(
                self.raw,
                SOL_SOCKET,
                SO_RCVBUF,
                (&raw mut value).cast::<c_void>(),
                &raw mut len,
            )
        };
        if rc == SOCKET_ERROR {
            return Err(NetError::last("getsockopt", self.endpoint));
        }
        Ok(value.max(0) as u32)
    }

    fn set_opt<T>(&self, level: i32, name: i32, value: &T, call: &'static str)
    -> Result<(), NetError> {
        // SAFETY: `value` is a live `T` and the length given is exactly its
        // size, which is what every option used here expects.
        let rc = unsafe {
            setsockopt(
                self.raw,
                level,
                name,
                (value as *const T).cast::<c_void>(),
                size_of::<T>() as i32,
            )
        };
        if rc == SOCKET_ERROR {
            return Err(NetError::last(call, self.endpoint));
        }
        Ok(())
    }
}

impl Drop for FeedSocket {
    fn drop(&mut self) {
        // SAFETY: the handle is valid and owned by this value; closing it
        // drops the group membership with it.
        unsafe { closesocket(self.raw) };
    }
}

/// `WSAPoll` over a fixed set of sockets.
///
/// Used by [`Mode::Block`](crate::recv::Mode) only. Its timeout **is** the
/// loop's tick: a cold feed that receives nothing still wakes up once per
/// timeout, which is when the heartbeat gets written
/// (`documents/todo.md` §5).
#[derive(Debug)]
pub struct Poller {
    fds: Vec<WsaPollFd>,
}

impl Poller {
    /// Watches these sockets for readable data, in the order given.
    pub fn new(sockets: &[FeedSocket]) -> Self {
        let fds = sockets
            .iter()
            .map(|s| WsaPollFd { fd: s.raw, events: POLLRDNORM, revents: 0 })
            .collect();
        Self { fds }
    }

    /// Waits up to `timeout_ms` and returns how many sockets are readable.
    ///
    /// With no sockets it returns `Ok(0)` without calling `WSAPoll`, which
    /// rejects an empty set — a feed configured with no sockets should idle,
    /// not fail on the first tick.
    pub fn wait(&mut self, timeout_ms: i32) -> Result<usize, NetError> {
        if self.fds.is_empty() {
            return Ok(0);
        }
        for fd in &mut self.fds {
            fd.revents = 0;
        }
        // SAFETY: `fds` is a live, non-empty array of `WSAPOLLFD` and the count
        // matches its length.
        let n = unsafe { WSAPoll(self.fds.as_mut_ptr(), self.fds.len() as u32, timeout_ms) };
        if n == SOCKET_ERROR {
            let e = NetError::last("WSAPoll", Endpoint::new(Ipv4Addr::UNSPECIFIED, 0));
            // A signalled wait is a wake-up, not a fault.
            if e.code == WSAEINTR {
                return Ok(0);
            }
            return Err(e);
        }
        Ok(n as usize)
    }

    /// `true` if the socket at `index` had data on the last
    /// [`wait`](Self::wait).
    #[inline]
    pub fn is_ready(&self, index: usize) -> bool {
        self.fds.get(index).is_some_and(|fd| fd.revents != 0)
    }

    /// Number of sockets watched.
    #[inline]
    pub fn len(&self) -> usize {
        self.fds.len()
    }

    /// `true` when no socket is watched.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.fds.is_empty()
    }
}

/// A Winsock call failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetError {
    /// The step that failed — a Winsock call, or one of our own checks, for
    /// locating it without a stack trace.
    pub call: &'static str,

    /// The endpoint being set up or read.
    pub endpoint: Endpoint,

    /// `WSAGetLastError()`. Zero when the failure was our own check rather
    /// than Winsock's.
    pub code: i32,
}

impl NetError {
    fn last(call: &'static str, endpoint: Endpoint) -> Self {
        // SAFETY: no arguments, no memory touched; reads this thread's last
        // Winsock error.
        Self { call, endpoint, code: unsafe { WSAGetLastError() } }
    }

    /// `true` for errors a receive loop should count and step over.
    ///
    /// `WSAECONNRESET` on a UDP socket reports an ICMP port-unreachable from an
    /// earlier send, and `WSAEMSGSIZE` reports one oversized datagram whose
    /// tail was discarded. Neither means the socket is unusable, and stopping
    /// the feed on either would trade the whole market for one packet.
    #[inline]
    pub const fn is_transient(&self) -> bool {
        matches!(self.code, WSAECONNRESET | WSAEMSGSIZE | WSAEINTR)
    }
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} failed on {}", self.call, self.endpoint)?;
        if self.code != 0 {
            write!(f, " (WSA {})", self.code)?;
        }
        Ok(())
    }
}

impl std::error::Error for NetError {}
