//! `conf/krx.toml`.
//!
//! ```toml
//! trcode_table = "conf/krx_trcodes.toml"
//!
//! [[feed]]
//! name = "hot"          mode = "spin"       cores = [2]
//! ring = "jeed.krx.hot" ring_slots = 65536
//! sockets = ["233.x.x.92:10302"]
//! trcodes = ["B601F", "G701F"]
//!
//! [filter]  isin = "conf/isin_allow.txt"
//! [health]  heartbeat_ms = 100   stale_ms = 500
//! [log]     report_secs = 10
//! ```
//!
//! ## What is checked, and what famously cannot be
//!
//! [`validate`](KrxConf::validate) refuses a conf that would run wrong: two
//! feeds on one port (both would receive both groups on Windows and publish
//! everything twice — `documents/todo.md` §15), two feeds on one core, a
//! spinning feed spread over several cores, a cold feed on a spinning feed's
//! SMT sibling, a ring that is not a power of two. It **cannot** check that a
//! socket carries the trcodes listed next to it — which port carries what is
//! a circuit assignment the standard does not state — and the runtime
//! `never seen` line is the substitute.
//!
//! [`warnings`](KrxConf::warnings) is for what might be wrong: a trcode the
//! standard's table does not list, a trcode this build has no decoder for, a
//! trcode listed under two feeds.

use crate::conf::{ConfError, Health, Section, TrCodeTable, read, trcodes::trcode};
use crate::cpu::{MAX_LOGICAL, Topology, mask_of};
use crate::toml::Table;
use core::fmt;
use jeed_krx::TrCode;
use jeed_krx::decode::dispatch;
use jeed_krx::recv::{Config, Endpoint, Mode, SocketOptions};
use jeed_shm::{SegmentName, ShmError};
use std::path::{Path, PathBuf};

/// The whole file.
#[derive(Debug, Clone, PartialEq)]
pub struct KrxConf {
    /// `trcode_table` — where `conf/krx_trcodes.toml` is, if validation
    /// against it is wanted.
    pub trcode_table: Option<PathBuf>,

    /// `[[feed]]`, in file order.
    pub feeds: Vec<FeedConf>,

    /// `[filter] isin` — the 종목코드 allow-list, if any.
    pub isin_list: Option<PathBuf>,

    /// `[health]`.
    pub health: Health,

    /// `[log] report_secs` — cadence of the per-feed report line. Zero
    /// silences it.
    pub report_secs: u64,
}

/// One `[[feed]]`: one thread, one ring.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedConf {
    /// `name` — for the thread and the log.
    pub name: String,

    /// `mode` — `"spin"` or `"block"`.
    pub mode: Mode,

    /// `cores` — the affinity set. Exactly one for a spinning feed.
    pub cores: Vec<u16>,

    /// `ring` — segment name, session-local namespace.
    pub ring: String,

    /// `ring_slots` — a power of two.
    pub ring_slots: u64,

    /// `sockets` — what to join.
    pub sockets: Vec<Endpoint>,

    /// `trcodes` — what to keep.
    pub trcodes: Vec<TrCode>,

    /// `burst` — datagrams one socket may yield per round.
    pub burst: u32,

    /// `recv_buffer_bytes` — `SO_RCVBUF`.
    pub recv_buffer_bytes: u32,
}

impl FeedConf {
    /// The receiver's configuration for this feed under `health`.
    pub fn receiver_config(&self, health: Health) -> Config {
        Config {
            mode: self.mode,
            heartbeat_ns: health.heartbeat_ns,
            stale_ns: health.stale_ns,
            burst: self.burst,
            socket: SocketOptions { recv_buffer_bytes: self.recv_buffer_bytes, ..SocketOptions::default() },
        }
    }

    /// The affinity mask, or `None` if a core is beyond [`MAX_LOGICAL`].
    #[inline]
    pub fn mask(&self) -> Option<u64> {
        mask_of(&self.cores)
    }
}

const FEED_KEYS: &[&str] =
    &["name", "mode", "cores", "ring", "ring_slots", "sockets", "trcodes", "burst", "recv_buffer_bytes"];
const ROOT_KEYS: &[&str] = &["trcode_table", "feed", "filter", "health", "log"];

impl KrxConf {
    /// Reads a file.
    pub fn load(path: &Path) -> Result<Self, ConfError> {
        Self::from_table(&read(path)?)
    }

    /// Reads parsed TOML.
    pub fn from_table(table: &Table) -> Result<Self, ConfError> {
        let root = Section::new("", table);

        let trcode_table = root.optional_str("trcode_table")?.map(PathBuf::from);

        let mut feeds = Vec::new();
        for (i, t) in root.tables("feed")?.into_iter().enumerate() {
            let at = format!("feed[{i}]");
            let section = Section::new(&at, t);
            let name = section.required_str("name")?.to_owned();
            let at = format!("feed[{name}]");
            let section = Section::new(&at, t);
            feeds.push(feed(&section, name)?);
        }

        let isin_list = match root.optional_table("filter")? {
            None => None,
            Some(s) => {
                let path = s.optional_str("isin")?.map(PathBuf::from);
                s.finish(&["isin"])?;
                path
            }
        };

        let health = Health::from_section(root.optional_table("health")?)?;

        let report_secs = match root.optional_table("log")? {
            None => 10,
            Some(s) => {
                let secs = s.optional_u64("report_secs")?.unwrap_or(10);
                s.finish(&["report_secs"])?;
                secs
            }
        };

        root.finish(ROOT_KEYS)?;
        Ok(Self { trcode_table, feeds, isin_list, health, report_secs })
    }

    /// Refuses a conf that would run wrong.
    ///
    /// `topology` is the box's SMT layout; without it the sibling rule is
    /// skipped (and [`warnings`](Self::warnings) says so).
    pub fn validate(&self, topology: Option<&Topology>) -> Result<(), RuleError> {
        if self.feeds.is_empty() {
            return Err(RuleError::NoFeeds);
        }

        for (i, feed) in self.feeds.iter().enumerate() {
            let name = &feed.name;
            if name.is_empty() {
                return Err(RuleError::EmptyName { index: i });
            }
            if let Some(other) = self.feeds[..i].iter().find(|f| f.name == *name) {
                return Err(RuleError::DuplicateName { name: other.name.clone() });
            }

            SegmentName::local(&feed.ring)
                .map_err(|source| RuleError::Ring { feed: name.clone(), source })?;
            if let Some(other) = self.feeds[..i].iter().find(|f| f.ring == feed.ring) {
                return Err(RuleError::DuplicateRing {
                    ring: feed.ring.clone(),
                    a: other.name.clone(),
                    b: name.clone(),
                });
            }
            if feed.ring_slots == 0 || !feed.ring_slots.is_power_of_two() {
                return Err(RuleError::RingSlots { feed: name.clone(), slots: feed.ring_slots });
            }

            if feed.cores.is_empty() {
                return Err(RuleError::NoCores { feed: name.clone() });
            }
            if feed.mode == Mode::Spin && feed.cores.len() != 1 {
                return Err(RuleError::SpinOnSeveralCores { feed: name.clone(), cores: feed.cores.len() });
            }
            let logical = topology.map_or(MAX_LOGICAL, Topology::logical);
            for &core in &feed.cores {
                if core as usize >= logical {
                    return Err(RuleError::CoreOutOfRange { feed: name.clone(), core, logical });
                }
                for other in &self.feeds[..i] {
                    if other.cores.contains(&core) {
                        return Err(RuleError::CoreShared {
                            core,
                            a: other.name.clone(),
                            b: name.clone(),
                        });
                    }
                }
            }

            if feed.sockets.is_empty() {
                return Err(RuleError::NoSockets { feed: name.clone() });
            }
            for (j, socket) in feed.sockets.iter().enumerate() {
                let earlier = feed.sockets[..j]
                    .iter()
                    .map(|s| (name, s))
                    .chain(self.feeds[..i].iter().flat_map(|f| f.sockets.iter().map(move |s| (&f.name, s))));
                for (other, s) in earlier {
                    if s.port == socket.port {
                        return Err(RuleError::PortShared {
                            port: socket.port,
                            a: other.clone(),
                            b: name.clone(),
                        });
                    }
                }
            }

            if feed.trcodes.is_empty() {
                return Err(RuleError::NoTrcodes { feed: name.clone() });
            }
            for (j, &code) in feed.trcodes.iter().enumerate() {
                if feed.trcodes[..j].contains(&code) {
                    return Err(RuleError::DuplicateTrcode { feed: name.clone(), code });
                }
            }
        }

        // The sibling rule needs every feed's cores, so it runs after the
        // per-feed pass.
        if let Some(topology) = topology {
            for spinning in self.feeds.iter().filter(|f| f.mode == Mode::Spin) {
                let mask = spinning.mask().expect("cores validated above");
                let forbidden = topology.siblings_outside(mask);
                for other in &self.feeds {
                    if other.name == spinning.name {
                        continue;
                    }
                    let other_mask = other.mask().expect("cores validated above");
                    if other_mask & forbidden != 0 {
                        let sibling = (other_mask & forbidden).trailing_zeros() as u16;
                        return Err(RuleError::SiblingShared {
                            spinning: spinning.name.clone(),
                            core: spinning.cores[0],
                            sibling,
                            other: other.name.clone(),
                        });
                    }
                }
            }
        }

        Ok(())
    }

    /// What might be wrong.
    ///
    /// `table` is the standard's trcode table; without it the "not in the
    /// table" check is skipped. `topology` is passed only so its absence can
    /// be reported alongside the rest.
    pub fn warnings(&self, table: Option<&TrCodeTable>, topology: Option<&Topology>) -> Vec<Warning> {
        let mut out = Vec::new();
        if topology.is_none() {
            out.push(Warning::NoTopology);
        }
        for (i, feed) in self.feeds.iter().enumerate() {
            for &code in &feed.trcodes {
                if let Some(table) = table
                    && !table.knows(code)
                {
                    out.push(Warning::NotInTable { feed: feed.name.clone(), code });
                }
                if !dispatch::handles(code) {
                    out.push(Warning::NoDecoder { feed: feed.name.clone(), code });
                }
                if let Some(other) = self.feeds[..i].iter().find(|f| f.trcodes.contains(&code)) {
                    out.push(Warning::TrcodeInTwoFeeds {
                        code,
                        a: other.name.clone(),
                        b: feed.name.clone(),
                    });
                }
            }
        }
        out
    }
}

fn feed(s: &Section<'_>, name: String) -> Result<FeedConf, ConfError> {
    let mode = match s.required_str("mode")? {
        "spin" => Mode::Spin,
        "block" => Mode::Block,
        other => return Err(s.invalid("mode", format!("must be \"spin\" or \"block\", found \"{other}\""))),
    };
    let cores = s.required_u16s("cores")?;
    let ring = s.required_str("ring")?.to_owned();
    let ring_slots = s.required_u64("ring_slots")?;

    let sockets = s
        .required_strs("sockets")?
        .into_iter()
        .map(|text| text.parse::<Endpoint>().map_err(|e| s.invalid("sockets", format!("`{text}`: {e}"))))
        .collect::<Result<Vec<_>, _>>()?;

    let trcodes = s
        .required_strs("trcodes")?
        .into_iter()
        .map(|text| trcode(s, text).map_err(|_| s.invalid("trcodes", format!("`{text}` is not a five-byte trcode"))))
        .collect::<Result<Vec<_>, _>>()?;

    let burst = match s.optional_u64("burst")? {
        None => Config::default().burst,
        Some(0) => return Err(s.invalid("burst", "must be at least 1")),
        Some(n) => u32::try_from(n).map_err(|_| s.invalid("burst", "is too large"))?,
    };
    let recv_buffer_bytes = match s.optional_u64("recv_buffer_bytes")? {
        None => SocketOptions::default().recv_buffer_bytes,
        Some(n) => u32::try_from(n).map_err(|_| s.invalid("recv_buffer_bytes", "is too large"))?,
    };

    s.finish(FEED_KEYS)?;
    Ok(FeedConf { name, mode, cores, ring, ring_slots, sockets, trcodes, burst, recv_buffer_bytes })
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

    /// `sockets` is empty.
    NoSockets {
        /// Feed.
        feed: String,
    },

    /// One port in two sockets — on Windows both would receive both groups.
    PortShared {
        /// The port.
        port: u16,
        /// First feed.
        a: String,
        /// Second feed.
        b: String,
    },

    /// `trcodes` is empty.
    NoTrcodes {
        /// Feed.
        feed: String,
    },

    /// One trcode twice in one feed.
    DuplicateTrcode {
        /// Feed.
        feed: String,
        /// The code.
        code: TrCode,
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
        }
    }
}

impl std::error::Error for RuleError {}

/// What might be wrong with a conf that will still run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// The SMT layout could not be read, so the sibling rule was not applied.
    NoTopology,

    /// A trcode neither standard lists — a typo, or a product newer than the
    /// standard.
    NotInTable {
        /// Feed.
        feed: String,
        /// The code.
        code: TrCode,
    },

    /// A trcode this build cannot decode. It will be counted as
    /// `unknown_trcode` and never published.
    NoDecoder {
        /// Feed.
        feed: String,
        /// The code.
        code: TrCode,
    },

    /// A trcode under two feeds. If both feeds' sockets carry it, it is
    /// published to both rings.
    TrcodeInTwoFeeds {
        /// The code.
        code: TrCode,
        /// First feed.
        a: String,
        /// Second feed.
        b: String,
    },
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoTopology => {
                write!(f, "SMT topology unavailable; the sibling-core rule was not checked")
            }
            Self::NotInTable { feed, code } => {
                write!(f, "feed[{feed}]: trcode {code} is in neither standard's table")
            }
            Self::NoDecoder { feed, code } => {
                write!(f, "feed[{feed}]: trcode {code} has no decoder in this build and will never be published")
            }
            Self::TrcodeInTwoFeeds { code, a, b } => {
                write!(f, "trcode {code} is listed under both \"{a}\" and \"{b}\"")
            }
        }
    }
}
