//! `host:port` — what the session connects to.
//!
//! ## Resolution happens at connect, not at parse
//!
//! [`Endpoint`] keeps the host as text and calls the resolver every time
//! [`Link::connect`](crate::recv::Link::connect) runs. A handler that resolved
//! once at boot and cached the address would keep dialling yesterday's box
//! after a venue failover, and would do it for the rest of the trading day —
//! the reconnect loop would look like it was working, because it *is* dialling
//! something.
//!
//! This is also the one place in the receive path where an allocation is fine:
//! it happens once per connection attempt, on the cold side of a session that
//! then runs for eight hours.
//!
//! ## The multicast check, inverted
//!
//! `jeed_krx::recv::Endpoint` refuses an address that is *not* multicast,
//! because a unicast group produces a socket that silently never receives.
//! Here the same class of mistake is a multicast address in a `connect`, which
//! fails at the OS rather than silently — so this type checks the shape of the
//! string and leaves the address to the resolver.

use core::fmt;
use core::str::FromStr;
use std::io;
use std::net::{SocketAddr, ToSocketAddrs};

/// The venue's market-data session address.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Endpoint {
    /// Host name or literal address, as configured.
    pub host: String,

    /// TCP port.
    pub port: u16,
}

impl Endpoint {
    /// An endpoint from its parts.
    #[inline]
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self { host: host.into(), port }
    }

    /// Resolves the host to a socket address, preferring IPv4.
    ///
    /// IPv4 first is not a preference about addressing; it is that a venue
    /// publishing both records usually has the session listener on the v4 one,
    /// and a handler that picks v6 and hangs at connect looks identical to a
    /// venue that is down.
    pub fn resolve(&self) -> io::Result<SocketAddr> {
        let mut first = None;
        for addr in (self.host.as_str(), self.port).to_socket_addrs()? {
            if addr.is_ipv4() {
                return Ok(addr);
            }
            first.get_or_insert(addr);
        }
        first.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("{self} resolved to no address"))
        })
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

impl FromStr for Endpoint {
    type Err = EndpointError;

    /// Parses `host:port`. An IPv6 literal is written in brackets, as
    /// everywhere else: `[::1]:9876`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let (host, port) = match s.strip_prefix('[') {
            Some(rest) => {
                let (host, tail) = rest.split_once(']').ok_or(EndpointError::Unbracketed)?;
                let port = tail.strip_prefix(':').ok_or(EndpointError::MissingPort)?;
                (host, port)
            }
            None => s.rsplit_once(':').ok_or(EndpointError::MissingPort)?,
        };
        if host.is_empty() {
            return Err(EndpointError::EmptyHost);
        }
        let port: u16 = port.parse().map_err(|_| EndpointError::BadPort)?;
        if port == 0 {
            return Err(EndpointError::BadPort);
        }
        Ok(Self { host: host.to_owned(), port })
    }
}

/// An endpoint string could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointError {
    /// No `:port` at all. A FIX endpoint without a port is not a default to
    /// guess at — there is no well-known FIX port.
    MissingPort,

    /// The port is not a number in `1..=65535`.
    BadPort,

    /// Nothing before the `:`.
    EmptyHost,

    /// An IPv6 literal opened with `[` and never closed.
    Unbracketed,
}

impl fmt::Display for EndpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPort => write!(f, "no `:port` — a FIX endpoint has no default port"),
            Self::BadPort => write!(f, "port is not a number in 1..=65535"),
            Self::EmptyHost => write!(f, "no host before the `:`"),
            Self::Unbracketed => write!(f, "`[` without a closing `]`"),
        }
    }
}

impl std::error::Error for EndpointError {}
