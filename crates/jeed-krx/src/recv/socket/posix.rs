//! BSD sockets on Linux.
//!
//! The eight entry points below are declared here rather than taken from a
//! bindings crate — see the module docs one level up.
//!
//! ## The bind is the filter here
//!
//! Unlike Windows, Linux accepts a bind to the **group address**, and that is
//! what this module does: the destination address then takes part in the
//! demultiplexing, so a socket receives its own group and nothing else. It also
//! sidesteps `IP_MULTICAST_ALL`, the knob that would otherwise hand an
//! `INADDR_ANY` socket every group anything on the host has joined on that
//! port.
//!
//! ## Two things the kernel does quietly
//!
//! `SO_RCVBUF` is **doubled** by the kernel for bookkeeping and reported back
//! doubled, and it is capped at `net.core.rmem_max` (commonly 208 KiB) unless
//! the process has `CAP_NET_ADMIN`. An 8 MiB request on a stock box therefore
//! grants far less than 8 MiB, which is exactly what
//! [`recv_buffer_bytes`](super::FeedSocket::recv_buffer_bytes) is for.
//!
//! A datagram larger than the read buffer is **truncated without an error** —
//! there is no `EMSGSIZE` on receive as there is on Windows. The truncated
//! message then fails the pipeline's length check, so it is rejected rather
//! than half-decoded; it is just counted as a wrong length instead of a socket
//! error.

use super::{OsErr, SocketOptions};
use crate::recv::endpoint::Endpoint;
use core::ffi::{c_int, c_short, c_void};

/// A file descriptor.
pub(super) type Raw = c_int;

const AF_INET: c_int = 2;
const SOCK_DGRAM: c_int = 2;
const IPPROTO_UDP: c_int = 17;
const IPPROTO_IP: c_int = 0;
const SOL_SOCKET: c_int = 1;
const SO_REUSEADDR: c_int = 2;
const SO_RCVBUF: c_int = 8;
const IP_ADD_MEMBERSHIP: c_int = 35;

const F_GETFL: c_int = 3;
const F_SETFL: c_int = 4;
const O_NONBLOCK: c_int = 0o4000;

const POLLIN: c_short = 0x001;

const EINTR: i32 = 4;
const EAGAIN: i32 = 11;
const ECONNREFUSED: i32 = 111;

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
struct PollFd {
    fd: Raw,
    events: c_short,
    revents: c_short,
}

const _: () = assert!(size_of::<SockAddrIn>() == 16);
const _: () = assert!(size_of::<IpMreq>() == 8);
const _: () = assert!(size_of::<PollFd>() == 8);

unsafe extern "C" {
    fn socket(domain: c_int, ty: c_int, protocol: c_int) -> c_int;
    fn bind(fd: c_int, addr: *const SockAddrIn, len: u32) -> c_int;
    fn setsockopt(fd: c_int, level: c_int, name: c_int, val: *const c_void, len: u32) -> c_int;
    fn getsockopt(fd: c_int, level: c_int, name: c_int, val: *mut c_void, len: *mut u32) -> c_int;
    /// Variadic in C. Declared with the one `int` argument every command used
    /// here takes, which is how the SysV call is made anyway.
    fn fcntl(fd: c_int, cmd: c_int, arg: c_int) -> c_int;
    #[link_name = "recv"]
    fn sys_recv(fd: c_int, buf: *mut u8, len: usize, flags: c_int) -> isize;
    #[link_name = "close"]
    fn sys_close(fd: c_int) -> c_int;
    fn poll(fds: *mut PollFd, nfds: u64, timeout: c_int) -> c_int;
    fn __errno_location() -> *mut i32;
}

fn errno() -> i32 {
    // SAFETY: glibc and musl both return a pointer to this thread's `errno`,
    // which is live for the life of the thread.
    unsafe { *__errno_location() }
}

fn last(call: &'static str) -> OsErr {
    (call, errno())
}

pub(super) fn join(endpoint: Endpoint, opts: SocketOptions) -> Result<Raw, OsErr> {
    // SAFETY: the arguments are the constants for a UDP/IPv4 datagram socket.
    let raw = unsafe { socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP) };
    if raw < 0 {
        return Err(last("socket"));
    }

    let setup = || -> Result<(), OsErr> {
        if opts.reuse_addr {
            set_opt(raw, SOL_SOCKET, SO_REUSEADDR, &1i32, "SO_REUSEADDR")?;
        }
        set_opt(raw, SOL_SOCKET, SO_RCVBUF, &(opts.recv_buffer_bytes as i32), "SO_RCVBUF")?;

        // The group address, not `INADDR_ANY` — see the module docs.
        let addr = SockAddrIn {
            sin_family: AF_INET as u16,
            sin_port: endpoint.port.to_be(),
            sin_addr: u32::from_ne_bytes(endpoint.group.octets()),
            sin_zero: [0; 8],
        };
        // SAFETY: `addr` is a live, fully initialised `sockaddr_in` and the
        // length matches its size.
        if unsafe { bind(raw, &raw const addr, size_of::<SockAddrIn>() as u32) } < 0 {
            return Err(last("bind"));
        }

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
    // SAFETY: `buf` is a live, writable slice of its own length, and `recv`
    // writes no more than that.
    let n = unsafe { sys_recv(raw, buf.as_mut_ptr(), buf.len(), 0) };
    if n >= 0 {
        return Ok(Some(n as usize));
    }
    let e = last("recv");
    if e.1 == EAGAIN { Ok(None) } else { Err(e) }
}

pub(super) fn set_nonblocking(raw: Raw, on: bool) -> Result<(), OsErr> {
    // SAFETY: `F_GETFL` takes no argument; the zero is ignored.
    let flags = unsafe { fcntl(raw, F_GETFL, 0) };
    if flags < 0 {
        return Err(last("fcntl"));
    }
    let next = if on { flags | O_NONBLOCK } else { flags & !O_NONBLOCK };
    // SAFETY: `F_SETFL` takes the flag word as its one argument.
    if unsafe { fcntl(raw, F_SETFL, next) } < 0 {
        return Err(last("fcntl"));
    }
    Ok(())
}

pub(super) fn recv_buffer_bytes(raw: Raw) -> Result<u32, OsErr> {
    let mut value: i32 = 0;
    let mut len = size_of::<i32>() as u32;
    // SAFETY: `value` and `len` are live; `len` states the buffer size and
    // `getsockopt` writes no more than that.
    let rc = unsafe {
        getsockopt(raw, SOL_SOCKET, SO_RCVBUF, (&raw mut value).cast::<c_void>(), &raw mut len)
    };
    if rc < 0 {
        return Err(last("getsockopt"));
    }
    Ok(value.max(0) as u32)
}

pub(super) fn close(raw: Raw) {
    // SAFETY: the descriptor is valid and owned by the caller, and closed once.
    unsafe { sys_close(raw) };
}

pub(super) fn is_transient(code: i32) -> bool {
    // No `EMSGSIZE`: Linux truncates an oversized datagram rather than
    // reporting it — see the module docs.
    matches!(code, ECONNREFUSED | EINTR)
}

fn set_opt<T>(raw: Raw, level: c_int, name: c_int, value: &T, call: &'static str)
-> Result<(), OsErr> {
    // SAFETY: `value` is a live `T` and the length given is exactly its size,
    // which is what every option used here expects.
    let rc = unsafe {
        setsockopt(raw, level, name, (value as *const T).cast::<c_void>(), size_of::<T>() as u32)
    };
    if rc < 0 {
        return Err(last(call));
    }
    Ok(())
}

/// `poll` over a fixed set of sockets.
#[derive(Debug)]
pub(super) struct Poll {
    fds: Vec<PollFd>,
}

impl Poll {
    pub(super) fn new(raws: impl Iterator<Item = Raw>) -> Self {
        let fds = raws.map(|fd| PollFd { fd, events: POLLIN, revents: 0 }).collect();
        Self { fds }
    }

    pub(super) fn wait(&mut self, timeout_ms: i32) -> Result<usize, OsErr> {
        // `poll` accepts an empty set, unlike `WSAPoll`; returning early keeps
        // the two platforms doing the same thing rather than one sleeping.
        if self.fds.is_empty() {
            return Ok(0);
        }
        for fd in &mut self.fds {
            fd.revents = 0;
        }
        // SAFETY: `fds` is a live array of `pollfd` and the count matches its
        // length.
        let n = unsafe { poll(self.fds.as_mut_ptr(), self.fds.len() as u64, timeout_ms) };
        if n < 0 {
            let e = last("poll");
            // A signalled wait is a wake-up, not a fault.
            if e.1 == EINTR {
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
