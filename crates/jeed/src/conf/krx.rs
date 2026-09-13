//! `conf/krx.toml`.
//!
//! ```toml
//! trcode_table = "conf/krx_trcodes.toml"
//!
//! [[feed]]
//! name = "hot"          mode = "spin"       cores = [2]
//! ring = "jeed.krx.hot" ring_slots = 16384
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

use crate::conf::rules::{Placement, check_placement};
use crate::conf::{ConfError, Health, Section, TrCodeTable, read, trcodes::trcode};
use crate::cpu::{Topology, mask_of};
use crate::toml::Table;
use core::fmt;
use jeed_krx::TrCode;
use jeed_krx::decode::dispatch;
use jeed_krx::recv::{Config, Endpoint, Mode, SocketOptions};
use std::path::{Path, PathBuf};

pub use crate::conf::rules::RuleError;

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

    /// The affinity mask, or `None` if a core is beyond
    /// [`MAX_LOGICAL`](crate::cpu::MAX_LOGICAL).
    #[inline]
    pub fn mask(&self) -> Option<u64> {
        mask_of(&self.cores)
    }

    /// The part every conf's feed has, for the shared rules.
    pub fn placement(&self) -> Placement<'_> {
        Placement {
            name: &self.name,
            spin: self.mode == Mode::Spin,
            cores: &self.cores,
            ring: &self.ring,
            ring_slots: self.ring_slots,
        }
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
        let placements: Vec<Placement<'_>> = self.feeds.iter().map(FeedConf::placement).collect();
        check_placement(&placements, topology)?;

        for (i, feed) in self.feeds.iter().enumerate() {
            let name = &feed.name;
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
