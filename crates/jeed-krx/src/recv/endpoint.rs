//! `(멀티캐스트 그룹IP, 포트)` — what one socket joins.
//!
//! **The socket does not identify the content.** A single 파생 product-group
//! port carries `B6` `G7` `A3` `V1` `Q2` `A7` `O6` … all mixed together, so an
//! endpoint says only *where to listen*; what to keep is the separate business
//! of [`TrCodeFilter`](crate::recv::TrCodeFilter). That split is why `conf`
//! spells `sockets` and `trcodes` as two lists (`documents/todo.md` §5).
//!
//! ## The interface is part of the address
//!
//! An IGMP join without an explicit local address goes out whichever interface
//! the routing table prefers, which on a box with a management NIC and a feed
//! NIC is a coin toss that silently produces an empty feed. `@` names the local
//! address to join on:
//!
//! ```text
//! 233.38.231.92:10302                 join on whatever the route picks
//! 233.38.231.92:10302@10.20.30.40     join on this NIC
//! ```

use core::fmt;
use core::str::FromStr;
use std::net::{AddrParseError, Ipv4Addr, SocketAddrV4};

/// A multicast group, its port, and the local interface to join on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Endpoint {
    /// 멀티캐스트 그룹IP. Must be in `224.0.0.0/4`.
    pub group: Ipv4Addr,

    /// 운용포트.
    pub port: u16,

    /// Local address to join on. [`Ipv4Addr::UNSPECIFIED`] lets the routing
    /// table choose — fine on a single-NIC box, a coin toss otherwise.
    pub interface: Ipv4Addr,
}

impl Endpoint {
    /// Endpoint joined on whichever interface the routing table picks.
    #[inline]
    pub const fn new(group: Ipv4Addr, port: u16) -> Self {
        Self { group, port, interface: Ipv4Addr::UNSPECIFIED }
    }

    /// The same endpoint joined on a named local address.
    #[inline]
    pub const fn on(self, interface: Ipv4Addr) -> Self {
        Self { interface, ..self }
    }

    /// `true` if the group address is actually a multicast address.
    ///
    /// A unicast address here is not an error the OS reports: `IP_ADD_MEMBERSHIP`
    /// fails, or worse, the bind succeeds and nothing ever arrives.
    #[inline]
    pub const fn group_is_multicast(&self) -> bool {
        self.group.is_multicast()
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.group, self.port)?;
        if !self.interface.is_unspecified() {
            write!(f, "@{}", self.interface)?;
        }
        Ok(())
    }
}

impl FromStr for Endpoint {
    type Err = EndpointError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (addr, iface) = match s.split_once('@') {
            Some((addr, iface)) => (addr, Some(iface)),
            None => (s, None),
        };

        let addr: SocketAddrV4 = addr.trim().parse().map_err(EndpointError::Address)?;
        if !addr.ip().is_multicast() {
            return Err(EndpointError::NotMulticast { group: *addr.ip() });
        }

        let interface = match iface {
            Some(i) => i.trim().parse().map_err(EndpointError::Interface)?,
            None => Ipv4Addr::UNSPECIFIED,
        };

        Ok(Self { group: *addr.ip(), port: addr.port(), interface })
    }
}

/// An endpoint string could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointError {
    /// The `group:port` half did not parse.
    Address(AddrParseError),

    /// The `@interface` half did not parse.
    Interface(AddrParseError),

    /// Syntactically an address, but not a multicast one — joining it would
    /// produce a socket that never receives.
    NotMulticast {
        /// The address given.
        group: Ipv4Addr,
    },
}

impl fmt::Display for EndpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Address(e) => write!(f, "not a `group:port` address: {e}"),
            Self::Interface(e) => write!(f, "not a local interface address: {e}"),
            Self::NotMulticast { group } => {
                write!(f, "{group} is not a multicast group (224.0.0.0/4)")
            }
        }
    }
}

impl std::error::Error for EndpointError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Address(e) | Self::Interface(e) => Some(e),
            Self::NotMulticast { .. } => None,
        }
    }
}
