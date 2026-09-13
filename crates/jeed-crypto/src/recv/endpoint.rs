//! `wss://host:port/path` — what the loop connects to.
//!
//! ## The path is part of the address
//!
//! On a FIX session the endpoint is `host:port` and everything else is said
//! in the Logon. A WebSocket subscription is often *in the URL* —
//! `wss://stream.binance.com:9443/stream?streams=btcusdt@trade` — so the path
//! and query are kept verbatim and sent in the `GET`. What to put there is
//! venue knowledge and belongs to the binary's conf, not here.
//!
//! ## Resolution happens at connect, not at parse
//!
//! As in `jeed_fix::recv::Endpoint`: the host stays text and is resolved on
//! every connection attempt, so a venue that fails over to another address
//! is followed by the reconnect loop rather than dialled at yesterday's box
//! all day.

use core::fmt;
use core::str::FromStr;
use std::io;
use std::net::{SocketAddr, ToSocketAddrs};

/// A WebSocket URL, taken apart.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Endpoint {
    /// Host name or literal address, as written.
    pub host: String,

    /// TCP port. Defaults to 443 for `wss` and 80 for `ws`.
    pub port: u16,

    /// Path and query, starting with `/`. `/` when the URL had none.
    pub path: String,

    /// `true` for `wss://`.
    pub tls: bool,
}

impl Endpoint {
    /// An endpoint from its parts.
    pub fn new(host: impl Into<String>, port: u16, path: impl Into<String>, tls: bool) -> Self {
        Self { host: host.into(), port, path: path.into(), tls }
    }

    /// The port the scheme implies when the URL names none.
    #[inline]
    pub const fn default_port(&self) -> u16 {
        if self.tls { 443 } else { 80 }
    }

    /// Resolves the host to a socket address, preferring IPv4.
    ///
    /// IPv4 first for the reason `jeed_fix` gives: a venue that publishes
    /// both records usually serves on the v4 one, and a handler that picks v6
    /// and hangs at connect looks identical to a venue that is down.
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
        let scheme = if self.tls { "wss" } else { "ws" };
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        write!(f, "{scheme}://{host}:{}{}", self.port, self.path)
    }
}

impl FromStr for Endpoint {
    type Err = EndpointError;

    /// Parses `ws://` or `wss://`, an optional port, and an optional path
    /// with query. An IPv6 literal is bracketed, as everywhere else.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let (tls, rest) = if let Some(r) = s.strip_prefix("wss://") {
            (true, r)
        } else if let Some(r) = s.strip_prefix("ws://") {
            (false, r)
        } else {
            return Err(EndpointError::Scheme);
        };

        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };

        let (host, port) = match authority.strip_prefix('[') {
            Some(inner) => {
                let (host, tail) = inner.split_once(']').ok_or(EndpointError::Unbracketed)?;
                let port = match tail.strip_prefix(':') {
                    Some(p) => Some(p),
                    None if tail.is_empty() => None,
                    None => return Err(EndpointError::BadPort),
                };
                (host, port)
            }
            None => match authority.rsplit_once(':') {
                Some((h, p)) => (h, Some(p)),
                None => (authority, None),
            },
        };
        if host.is_empty() {
            return Err(EndpointError::EmptyHost);
        }

        let default = if tls { 443 } else { 80 };
        let port = match port {
            None => default,
            Some(p) => {
                let p: u16 = p.parse().map_err(|_| EndpointError::BadPort)?;
                if p == 0 {
                    return Err(EndpointError::BadPort);
                }
                p
            }
        };

        Ok(Self { host: host.to_owned(), port, path: path.to_owned(), tls })
    }
}

/// A WebSocket URL could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointError {
    /// Not `ws://` or `wss://`. An `https://` here is a REST URL in the wrong
    /// slot, and that is worth refusing rather than guessing at.
    Scheme,

    /// Nothing between the scheme and the path.
    EmptyHost,

    /// The port is not a number in `1..=65535`.
    BadPort,

    /// An IPv6 literal opened with `[` and never closed.
    Unbracketed,
}

impl fmt::Display for EndpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scheme => write!(f, "URL must start with `ws://` or `wss://`"),
            Self::EmptyHost => write!(f, "no host after the scheme"),
            Self::BadPort => write!(f, "port is not a number in 1..=65535"),
            Self::Unbracketed => write!(f, "`[` without a closing `]`"),
        }
    }
}

impl std::error::Error for EndpointError {}
