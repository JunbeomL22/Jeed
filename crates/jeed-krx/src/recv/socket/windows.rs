//! Winsock.
//!
//! The nine entry points below are declared here rather than taken from a
//! bindings crate — see the module docs one level up.

use super::{OsErr, SocketOptions};
use crate::recv::endpoint::Endpoint;
use core::ffi::c_void;
use std::sync::Once;

/// Windows `SOCKET` — a `UINT_PTR`, not a file descriptor.
pub(super) type Raw = usize;

const INVALID_SOCKET: Raw = Raw::MAX;
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
    fd: Raw,
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
    fn socket(af: i32, ty: i32, protocol: i32) -> Raw;
    fn bind(s: Raw, name: *const SockAddrIn, namelen: i32) -> i32;
    fn setsockopt(s: Raw, level: i32, optname: i32, optval: *const c_void, optlen: i32) -> i32;
    fn getsockopt(s: Raw, level: i32, optname: i32, optval: *mut c_void, optlen: *mut i32) -> i32;
    fn ioctlsocket(s: Raw, cmd: i32, argp: *mut u32) -> i32;
    #[link_name = "recv"]
    fn sys_recv(s: Raw, buf: *mut u8, len: i32, flags: i32) -> i32;
    fn closesocket(s: Raw) -> i32;
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

fn last(call: &'static str) -> OsErr {
    // SAFETY: no arguments, no memory touched.
    (call, unsafe { WSAGetLastError() })
}

pub(super) fn join(endpoint: Endpoint, opts: SocketOptions) -> Result<Raw, OsErr> {
    winsock_init();

    // SAFETY: the arguments are the constants Winsock defines for a UDP/IPv4
    // datagram socket.
    let raw = unsafe { socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP) };
    if raw == INVALID_SOCKET {
        return Err(last("socket"));
    }

    let setup = || -> Result<(), OsErr> {
        if opts.reuse_addr {
            set_opt(raw, SOL_SOCKET, SO_REUSEADDR, &1i32, "SO_REUSEADDR")?;
        }
        set_opt(raw, SOL_SOCKET, SO_RCVBUF, &(opts.recv_buffer_bytes as i32), "SO_RCVBUF")?;

        // Windows refuses a bind to the group address (see the module docs),
        // so the socket binds to the local interface it joins on — `INADDR_ANY`
        // when none was named. The group is selected by the membership below;
        // the bind contributes the port and, with an interface, keeps the
        // socket off every other NIC (and off the firewall's radar when that
        // interface is loopback, which is how the tests run).
        let addr = SockAddrIn {
            sin_family: AF_INET as u16,
            sin_port: endpoint.port.to_be(),
            sin_addr: u32::from_ne_bytes(endpoint.interface.octets()),
            sin_zero: [0; 8],
        };
        // SAFETY: `addr` is a live, fully initialised `sockaddr_in` and the
        // length matches its size.
        if unsafe { bind(raw, &raw const addr, size_of::<SockAddrIn>() as i32) } == SOCKET_ERROR {
            return Err(last("bind"));
        }

        // The membership must follow the bind on Windows.
        let mreq = IpMreq {
            imr_multiaddr: u32::from_ne_bytes(endpoint.group.octets()),
            imr_interface: u32::from_ne_bytes(endpoint.interface.octets()),
        };
        set_opt(raw, IPPROTO_IP, IP_ADD_MEMBERSHIP, &mreq, "IP_ADD_MEMBERSHIP")?;

        if opts.nonblocking {
            set_nonblocking(raw, true)?;
        }
        Ok(())
    };

    match setup() {
        Ok(()) => Ok(raw),
        Err(e) => {
            close(raw);
            Err(e)
        }
    }
}

pub(super) fn recv(raw: Raw, buf: &mut [u8]) -> Result<Option<usize>, OsErr> {
    let len = buf.len().min(i32::MAX as usize) as i32;
    // SAFETY: `buf` is a live, writable slice of at least `len` bytes, and
    // `recv` writes no more than that.
    let n = unsafe { sys_recv(raw, buf.as_mut_ptr(), len, 0) };
    if n >= 0 {
        return Ok(Some(n as usize));
    }
    let e = last("recv");
    if e.1 == WSAEWOULDBLOCK { Ok(None) } else { Err(e) }
}

pub(super) fn set_nonblocking(raw: Raw, on: bool) -> Result<(), OsErr> {
    let mut flag: u32 = u32::from(on);
    // SAFETY: `FIONBIO` takes a pointer to one `u_long`, which is what `flag`
    // is.
    if unsafe { ioctlsocket(raw, FIONBIO, &raw mut flag) } == SOCKET_ERROR {
        return Err(last("ioctlsocket"));
    }
    Ok(())
}

pub(super) fn recv_buffer_bytes(raw: Raw) -> Result<u32, OsErr> {
    let mut value: i32 = 0;
    let mut len = size_of::<i32>() as i32;
    // SAFETY: `value` and `len` are live; `len` states the buffer size and
    // `getsockopt` writes no more than that.
    let rc = unsafe {
        getsockopt(raw, SOL_SOCKET, SO_RCVBUF, (&raw mut value).cast::<c_void>(), &raw mut len)
    };
    if rc == SOCKET_ERROR {
        return Err(last("getsockopt"));
    }
    Ok(value.max(0) as u32)
}

pub(super) fn close(raw: Raw) {
    // SAFETY: the handle is valid and owned by the caller, and closed once.
    unsafe { closesocket(raw) };
}

pub(super) fn is_transient(code: i32) -> bool {
    matches!(code, WSAECONNRESET | WSAEMSGSIZE | WSAEINTR)
}

fn set_opt<T>(raw: Raw, level: i32, name: i32, value: &T, call: &'static str) -> Result<(), OsErr> {
    // SAFETY: `value` is a live `T` and the length given is exactly its size,
    // which is what every option used here expects.
    let rc = unsafe {
        setsockopt(raw, level, name, (value as *const T).cast::<c_void>(), size_of::<T>() as i32)
    };
    if rc == SOCKET_ERROR {
        return Err(last(call));
    }
    Ok(())
}

/// `WSAPoll` over a fixed set of sockets.
#[derive(Debug)]
pub(super) struct Poll {
    fds: Vec<WsaPollFd>,
}

impl Poll {
    pub(super) fn new(raws: impl Iterator<Item = Raw>) -> Self {
        let fds = raws.map(|fd| WsaPollFd { fd, events: POLLRDNORM, revents: 0 }).collect();
        Self { fds }
    }

    pub(super) fn wait(&mut self, timeout_ms: i32) -> Result<usize, OsErr> {
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
            let e = last("WSAPoll");
            // A signalled wait is a wake-up, not a fault.
            if e.1 == WSAEINTR {
                return Ok(0);
            }
            return Err(e);
        }
        Ok(n as usize)
    }

    pub(super) fn is_ready(&self, index: usize) -> bool {
        self.fds.get(index).is_some_and(|fd| fd.revents != 0)
    }

    pub(super) fn len(&self) -> usize {
        self.fds.len()
    }
}
