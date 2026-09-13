//! Rules that span keys or sections — and the ones every feed conf shares.
//!
//! A KRX feed and a crypto feed are placed the same way: a name, a mode, a
//! set of cores, a ring. So the rules about *placement* — two feeds on one
//! core, a spinning feed spread over several, a cold feed on a spinning
//! feed's SMT sibling, a ring that is not a power of two — are checked once,
//! here, from a [`Placement`] view of whichever conf it is. What is checked
//! about sockets, trcodes or instruments stays with the conf that has them.
//!
//! [`RuleError`] is one enum for every conf rather than one per conf, so a
//! binary reports "feeds \"a\" and \"b\" both use core 2" in the same words
//! whichever handler it is.

use crate::cpu::{MAX_LOGICAL, Topology, mask_of};
use core::fmt;
use jeed_crypto::CryptoError;
use jeed_crypto::recv::RouterError;
use jeed_krx::TrCode;
use jeed_shm::{SegmentName, ShmError};

/// The part of a feed every conf has.
#[derive(Debug, Clone, Copy)]
pub struct Placement<'a> {
    /// `name`.
    pub name: &'a str,

    /// `true` for `mode = "spin"`.
    pub spin: bool,

    /// `cores`.
    pub cores: &'a [u16],

    /// `ring`.
    pub ring: &'a str,

    /// `ring_slots`.
    pub ring_slots: u64,
}

impl Placement<'_> {
    /// The affinity mask, or `None` if a core is beyond [`MAX_LOGICAL`].
    #[inline]
    pub fn mask(&self) -> Option<u64> {
        mask_of(self.cores)
    }
}

/// Refuses a set of feeds that would run wrong on this box.
///
/// `topology` is the box's SMT layout; without it the sibling rule is
/// skipped (and the conf's `warnings` says so).
pub fn check_placement(feeds: &[Placement<'_>], topology: Option<&Topology>) -> Result<(), RuleError> {
    if feeds.is_empty() {
        return Err(RuleError::NoFeeds);
    }

    for (i, feed) in feeds.iter().enumerate() {
        let name = feed.name;
        if name.is_empty() {
            return Err(RuleError::EmptyName { index: i });
        }
        if let Some(other) = feeds[..i].iter().find(|f| f.name == name) {
            return Err(RuleError::DuplicateName { name: other.name.to_owned() });
        }

        SegmentName::local(feed.ring).map_err(|source| RuleError::Ring { feed: name.to_owned(), source })?;
        if let Some(other) = feeds[..i].iter().find(|f| f.ring == feed.ring) {
            return Err(RuleError::DuplicateRing {
                ring: feed.ring.to_owned(),
                a: other.name.to_owned(),
                b: name.to_owned(),
            });
        }
        if feed.ring_slots == 0 || !feed.ring_slots.is_power_of_two() {
            return Err(RuleError::RingSlots { feed: name.to_owned(), slots: feed.ring_slots });
        }

        if feed.cores.is_empty() {
            return Err(RuleError::NoCores { feed: name.to_owned() });
        }
        if feed.spin && feed.cores.len() != 1 {
            return Err(RuleError::SpinOnSeveralCores { feed: name.to_owned(), cores: feed.cores.len() });
        }
        let logical = topology.map_or(MAX_LOGICAL, Topology::logical);
        for &core in feed.cores {
            if core as usize >= logical {
                return Err(RuleError::CoreOutOfRange { feed: name.to_owned(), core, logical });
            }
            for other in &feeds[..i] {
                if other.cores.contains(&core) {
                    return Err(RuleError::CoreShared { core, a: other.name.to_owned(), b: name.to_owned() });
                }
            }
        }
    }

    // The sibling rule needs every feed's cores, so it runs after the
    // per-feed pass.
    if let Some(topology) = topology {
        for spinning in feeds.iter().filter(|f| f.spin) {
            let mask = spinning.mask().expect("cores validated above");
            let forbidden = topology.siblings_outside(mask);
            for other in feeds {
                if other.name == spinning.name {
                    continue;
                }
                let other_mask = other.mask().expect("cores validated above");
                if other_mask & forbidden != 0 {
                    let sibling = (other_mask & forbidden).trailing_zeros() as u16;
                    return Err(RuleError::SiblingShared {
                        spinning: spinning.name.to_owned(),
                        core: spinning.cores[0],
                        sibling,
                        other: other.name.to_owned(),
                    });
                }
            }
        }
    }

    Ok(())
}

/// A rule spanning keys or sections was broken — the conf would run wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleError {
    /// No `[[feed]]` at all.
    NoFeeds,

    /// A feed with an empty `name`.
    EmptyName {
        /// Position in the file, zero-based.
        index: usize,
    },

    /// Two feeds with one `name`.
    DuplicateName {
        /// The name.
        name: String,
    },

    /// A `ring` the OS would not accept as an object name.
    Ring {
        /// Feed.
        feed: String,
        /// Why not.
        source: ShmError,
    },

    /// Two feeds writing one segment — the ring is single-producer.
    DuplicateRing {
        /// The segment.
        ring: String,
        /// First feed.
        a: String,
        /// Second feed.
        b: String,
    },

    /// `ring_slots` is zero or not a power of two.
    RingSlots {
        /// Feed.
        feed: String,
        /// What was asked.
        slots: u64,
    },

    /// `cores` is empty.
    NoCores {
        /// Feed.
        feed: String,
    },

    /// A spinning feed with more than one core: the thread would migrate,
    /// and "one pinned core" would be a figure of speech.
    SpinOnSeveralCores {
        /// Feed.
        feed: String,
        /// How many were listed.
        cores: usize,
    },

    /// A core the box does not have.
    CoreOutOfRange {
        /// Feed.
        feed: String,
        /// The core.
        core: u16,
        /// Logical processors on the box.
        logical: usize,
    },

    /// One core in two feeds.
    CoreShared {
        /// The core.
        core: u16,
        /// First feed.
        a: String,
        /// Second feed.
        b: String,
    },

    /// A feed on the SMT sibling of a spinning feed's core.
    SiblingShared {
        /// The spinning feed.
        spinning: String,
        /// Its core.
        core: u16,
        /// The sibling.
        sibling: u16,
        /// The feed that would share the physical core.
        other: String,
    },

    /// `sockets` is empty (KRX).
    NoSockets {
        /// Feed.
        feed: String,
    },

    /// One port in two sockets — on Windows both would receive both groups
    /// (KRX).
    PortShared {
        /// The port.
        port: u16,
        /// First feed.
        a: String,
        /// Second feed.
        b: String,
    },

    /// `trcodes` is empty (KRX).
    NoTrcodes {
        /// Feed.
        feed: String,
    },

    /// One trcode twice in one feed (KRX).
    DuplicateTrcode {
        /// Feed.
        feed: String,
        /// The code.
        code: TrCode,
    },

    /// No `[[feed.instrument]]` (crypto).
    NoInstruments {
        /// Feed.
        feed: String,
    },

    /// A symbol or scale the wire cannot carry (crypto).
    Instrument {
        /// Feed.
        feed: String,
        /// The symbol.
        symbol: String,
        /// Why.
        source: CryptoError,
    },

    /// The venue's router refused the feed's instruments — a channel the
    /// venue has no stream for, a depth it does not offer (crypto).
    Router {
        /// Feed.
        feed: String,
        /// Why.
        source: RouterError,
    },
}

impl fmt::Display for RuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoFeeds => write!(f, "no [[feed]] section"),
            Self::EmptyName { index } => write!(f, "feed[{index}]: `name` is empty"),
            Self::DuplicateName { name } => write!(f, "two feeds are named \"{name}\""),
            Self::Ring { feed, source } => write!(f, "feed[{feed}]: `ring`: {source}"),
            Self::DuplicateRing { ring, a, b } => {
                write!(f, "feeds \"{a}\" and \"{b}\" both write ring \"{ring}\"; a ring has one producer")
            }
            Self::RingSlots { feed, slots } => {
                write!(f, "feed[{feed}]: `ring_slots` = {slots} is not a power of two")
            }
            Self::NoCores { feed } => write!(f, "feed[{feed}]: `cores` is empty"),
            Self::SpinOnSeveralCores { feed, cores } => {
                write!(f, "feed[{feed}]: a spinning feed pins to exactly one core, not {cores}")
            }
            Self::CoreOutOfRange { feed, core, logical } => {
                write!(f, "feed[{feed}]: core {core} does not exist ({logical} logical processors)")
            }
            Self::CoreShared { core, a, b } => {
                write!(f, "feeds \"{a}\" and \"{b}\" both use core {core}")
            }
            Self::SiblingShared { spinning, core, sibling, other } => write!(
                f,
                "feed \"{other}\" uses core {sibling}, the SMT sibling of core {core} that \"{spinning}\" spins on"
            ),
            Self::NoSockets { feed } => write!(f, "feed[{feed}]: `sockets` is empty"),
            Self::PortShared { port, a, b } => write!(
                f,
                "feeds \"{a}\" and \"{b}\" both listen on port {port}; on Windows each socket would receive both groups"
            ),
            Self::NoTrcodes { feed } => write!(f, "feed[{feed}]: `trcodes` is empty"),
            Self::DuplicateTrcode { feed, code } => {
                write!(f, "feed[{feed}]: trcode {code} is listed twice")
            }
            Self::NoInstruments { feed } => write!(f, "feed[{feed}]: no [[feed.instrument]]"),
            Self::Instrument { feed, symbol, source } => {
                write!(f, "feed[{feed}]: instrument {symbol}: {source}")
            }
            Self::Router { feed, source } => write!(f, "feed[{feed}]: {source}"),
        }
    }
}

impl std::error::Error for RuleError {}
