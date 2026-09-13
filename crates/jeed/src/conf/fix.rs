//! `conf/fix.toml`.
//!
//! ```toml
//! [[feed]]
//! name = "smbs"   venue = "smbs"   mode = "block"   cores = [4]
//! ring = "jeed.fix.smbs"   ring_slots = 16384
//! host = "10.0.0.1"   port = 9100
//! sender_comp_id = "JEED"   target_comp_id = "SMBS"
//! price_decimals = 2   qty_decimals = 0
//! symbols = ["USD/KRW", "EUR/KRW"]
//! # begin_string = "FIX.4.4"  heartbeat_secs = 30  depth = 0  subscription = "updates"
//! # reset_seq = true  reconnect_secs = 5  logon_timeout_secs = 10  burst = 64  default_qty = 0
//!
//! [health]  heartbeat_ms = 100   stale_ms = 500
//! [log]     report_secs = 10
//! ```
//!
//! ## One feed = one session = one ring
//!
//! A `[[feed]]` is one FIX session to one venue, and everything it publishes
//! goes to one ring — the same shape as a KRX feed or a crypto connection
//! ([`crypto`](super::crypto)). Two venues are two feeds.
//!
//! ## The venue layer is a conf-driven adapter, not venue code
//!
//! `jeed-fix` knows FIX and no venue (`documents/feed_handler.md` §13); the
//! step from a decoded `35=W`/`35=X` to a wire record is an
//! [`MdAdapter`](jeed_fix::recv::MdAdapter). This conf drives
//! [`SnapshotAdapter`](crate::fix::SnapshotAdapter): the venue byte, the
//! price and size scales, and the symbols all come from here, and the
//! adapter maps a full refresh to a book snapshot and a trade to a trade —
//! **refusing** anything ambiguous (an incremental that moves the book, a
//! level with no size and no configured default) rather than guessing. The
//! guesses that are genuinely venue-specific — whether SMBS sends `35=X` book
//! moves at all, what an absent size means there — are the venue's to settle
//! when the session is confirmed live, and until then a refusal is a counted
//! `fail`, not a wrong book.
//!
//! ## Scales are one per session
//!
//! A FIX message carries no precision, so the decoder scales every price and
//! size once, at parse, with the session's [`FixScales`]
//! (`documents/feed_handler.md` §13). They are a venue fact from conf, like a
//! crypto instrument's `price_decimals`, and getting one wrong is a refused
//! frame rather than a rounded price.

use crate::conf::rules::{Placement, RuleError, check_placement};
use crate::conf::{ConfError, Health, Section, read};
use crate::cpu::{Topology, mask_of};
use crate::toml::Table;
use core::fmt;
use jeed_fix::FixScales;
use jeed_fix::recv::{Config, Endpoint, LinkOptions, Mode, SubscriptionType};
use jeed_wire::{Scale, Venue, symbol_from_bytes};
use std::path::Path;

/// The whole file.
#[derive(Debug, Clone, PartialEq)]
pub struct FixConf {
    /// `[[feed]]`, in file order.
    pub feeds: Vec<FixFeedConf>,

    /// `[health]`.
    pub health: Health,

    /// `[log] report_secs`. Zero silences the report line.
    pub report_secs: u64,
}

/// One `[[feed]]`: one FIX session, one thread, one ring.
#[derive(Debug, Clone, PartialEq)]
pub struct FixFeedConf {
    /// `name` — for the thread and the log.
    pub name: String,

    /// `venue` — which venue speaks this session, by conf name
    /// ([`venue_by_name`]).
    pub venue: Venue,

    /// `mode` — `"spin"` or `"block"`.
    pub mode: Mode,

    /// `cores` — the affinity set. Exactly one for a spinning feed.
    pub cores: Vec<u16>,

    /// `ring` — segment name.
    pub ring: String,

    /// `ring_slots` — a power of two.
    pub ring_slots: u64,

    /// `host` / `port` — where to connect.
    pub host: String,

    /// `port`.
    pub port: u16,

    /// `sender_comp_id` (49) — us.
    pub sender_comp_id: String,

    /// `target_comp_id` (56) — the venue.
    pub target_comp_id: String,

    /// `begin_string` — the FIX version string in the Logon, default
    /// `FIX.4.4`.
    pub begin_string: String,

    /// `price_decimals` — the session's price scale.
    pub price_decimals: usize,

    /// `qty_decimals` — the session's size scale.
    pub qty_decimals: usize,

    /// `default_qty` — the size to publish for a snapshot level that carries
    /// none, on the session's size scale. `None` refuses such a level rather
    /// than inventing one.
    pub default_qty: Option<u64>,

    /// `symbols` — the instruments to request and keep.
    pub symbols: Vec<String>,

    /// `depth` (264) — market depth to request. `0` is the full book.
    pub depth: u32,

    /// `subscription` — snapshot once, or a snapshot and every update after.
    pub subscription: SubscriptionType,

    /// `heartbeat_secs` (108).
    pub heartbeat_secs: u32,

    /// `reset_seq` (141=Y with the Logon).
    pub reset_seq: bool,

    /// `reconnect_secs` — wait between connection attempts.
    pub reconnect_secs: u64,

    /// `logon_timeout_secs` — how long to wait for the venue's Logon.
    pub logon_timeout_secs: u64,

    /// `burst` — messages framed per round.
    pub burst: u32,
}

impl FixFeedConf {
    /// The receiver's configuration under `health`.
    pub fn receiver_config(&self, health: Health) -> Config {
        Config {
            mode: self.mode,
            heartbeat_secs: self.heartbeat_secs,
            reset_seq_on_logon: self.reset_seq,
            record_heartbeat_ns: health.heartbeat_ns,
            stale_ns: health.stale_ns,
            burst: self.burst,
            reconnect_ns: self.reconnect_secs * 1_000_000_000,
            logon_timeout_ns: self.logon_timeout_secs * 1_000_000_000,
            link: LinkOptions::default(),
        }
    }

    /// The affinity mask, or `None` if a core is beyond the mask width.
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

    /// Where to connect.
    pub fn endpoint(&self) -> Endpoint {
        Endpoint::new(self.host.clone(), self.port)
    }

    /// The session's price and size scales.
    ///
    /// Infallible: the decimals were bounded to `0..=8` at parse, which every
    /// [`Scale`] admits.
    pub fn scales(&self) -> FixScales {
        FixScales::new(
            Scale::from_decimals(self.price_decimals).expect("0..=8"),
            Scale::from_decimals(self.qty_decimals).expect("0..=8"),
        )
    }
}

const FEED_KEYS: &[&str] = &[
    "name",
    "venue",
    "mode",
    "cores",
    "ring",
    "ring_slots",
    "host",
    "port",
    "sender_comp_id",
    "target_comp_id",
    "begin_string",
    "price_decimals",
    "qty_decimals",
    "default_qty",
    "symbols",
    "depth",
    "subscription",
    "heartbeat_secs",
    "reset_seq",
    "reconnect_secs",
    "logon_timeout_secs",
    "burst",
];
const ROOT_KEYS: &[&str] = &["feed", "health", "log"];

/// Conf names and the venues that speak FIX.
///
/// One entry today. FIX is a protocol, not a place, so this grows by a row
/// and an [`MdAdapter`](jeed_fix::recv::MdAdapter) — not by a crate — the day
/// a second venue is reached over it (`documents/feed_handler.md` §13).
const VENUES: &[(&str, Venue)] = &[("smbs", Venue::Smbs)];

/// The venue a conf `venue = "…"` names.
pub fn venue_by_name(name: &str) -> Option<Venue> {
    VENUES.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
}

/// Every conf venue name, for the error message.
pub fn venue_names() -> impl Iterator<Item = &'static str> {
    VENUES.iter().map(|(n, _)| *n)
}

/// The conf name of a FIX venue, or `"?"` for one with none.
pub fn venue_name_or(venue: Venue) -> &'static str {
    VENUES.iter().find(|(_, v)| *v == venue).map_or("?", |(n, _)| *n)
}

impl FixConf {
    /// Reads a file.
    pub fn load(path: &Path) -> Result<Self, ConfError> {
        Self::from_table(&read(path)?)
    }

    /// Reads parsed TOML.
    pub fn from_table(table: &Table) -> Result<Self, ConfError> {
        let root = Section::new("", table);

        let mut feeds = Vec::new();
        for (i, t) in root.tables("feed")?.into_iter().enumerate() {
            let at = format!("feed[{i}]");
            let section = Section::new(&at, t);
            let name = section.required_str("name")?.to_owned();
            let at = format!("feed[{name}]");
            let section = Section::new(&at, t);
            feeds.push(feed(&section, name)?);
        }

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
        Ok(Self { feeds, health, report_secs })
    }

    /// Refuses a conf that would run wrong.
    pub fn validate(&self, topology: Option<&Topology>) -> Result<(), RuleError> {
        let placements: Vec<Placement<'_>> = self.feeds.iter().map(FixFeedConf::placement).collect();
        check_placement(&placements, topology)?;
        for feed in &self.feeds {
            if feed.symbols.is_empty() {
                return Err(RuleError::NoInstruments { feed: feed.name.clone() });
            }
        }
        Ok(())
    }

    /// What might be wrong with a conf that will still run.
    pub fn warnings(&self, topology: Option<&Topology>) -> Vec<Warning> {
        let mut out = Vec::new();
        if topology.is_none() {
            out.push(Warning::NoTopology);
        }
        for (i, feed) in self.feeds.iter().enumerate() {
            for symbol in &feed.symbols {
                if let Some(other) = self.feeds[..i]
                    .iter()
                    .find(|f| f.venue == feed.venue && f.symbols.contains(symbol))
                {
                    out.push(Warning::SymbolInTwoFeeds {
                        venue: feed.venue,
                        symbol: symbol.clone(),
                        a: other.name.clone(),
                        b: feed.name.clone(),
                    });
                }
            }
        }
        out
    }
}

fn feed(s: &Section<'_>, name: String) -> Result<FixFeedConf, ConfError> {
    let venue_text = s.required_str("venue")?;
    let venue = venue_by_name(venue_text).ok_or_else(|| {
        let known: Vec<&str> = venue_names().collect();
        s.invalid("venue", format!("\"{venue_text}\" is not a FIX venue; one of {}", known.join(", ")))
    })?;
    let mode = match s.required_str("mode")? {
        "spin" => Mode::Spin,
        "block" => Mode::Block,
        other => return Err(s.invalid("mode", format!("must be \"spin\" or \"block\", found \"{other}\""))),
    };
    let cores = s.required_u16s("cores")?;
    let ring = s.required_str("ring")?.to_owned();
    let ring_slots = s.required_u64("ring_slots")?;

    let host = s.required_str("host")?.to_owned();
    let port = u16::try_from(s.required_u64("port")?).map_err(|_| s.invalid("port", "must be 1..=65535"))?;
    if port == 0 {
        return Err(s.invalid("port", "must be 1..=65535"));
    }
    let sender_comp_id = s.required_str("sender_comp_id")?.to_owned();
    let target_comp_id = s.required_str("target_comp_id")?.to_owned();
    let begin_string = s.optional_str("begin_string")?.unwrap_or("FIX.4.4").to_owned();

    let decimals = |key: &'static str| -> Result<usize, ConfError> {
        let n = s.required_u64(key)?;
        usize::try_from(n).ok().filter(|&n| n <= 8).ok_or_else(|| s.invalid(key, "must be 0..=8"))
    };
    let price_decimals = decimals("price_decimals")?;
    let qty_decimals = decimals("qty_decimals")?;
    let default_qty = s.optional_u64("default_qty")?;

    let symbols: Vec<String> = s.required_strs("symbols")?.into_iter().map(str::to_owned).collect();
    for symbol in &symbols {
        if symbol.is_empty() {
            return Err(s.invalid("symbols", "a symbol is empty"));
        }
        if symbol_from_bytes(symbol.as_bytes()).is_none() {
            return Err(s.invalid("symbols", format!("`{symbol}` is longer than the wire symbol field")));
        }
    }

    let depth = match s.optional_u64("depth")? {
        None => 0,
        Some(n) => u32::try_from(n).map_err(|_| s.invalid("depth", "is too large"))?,
    };
    let subscription = match s.optional_str("subscription")? {
        None | Some("updates") => SubscriptionType::SnapshotPlusUpdates,
        Some("snapshot") => SubscriptionType::Snapshot,
        Some(other) => {
            return Err(s.invalid("subscription", format!("must be \"updates\" or \"snapshot\", found \"{other}\"")));
        }
    };

    let d = Config::default();
    let heartbeat_secs = match s.optional_u64("heartbeat_secs")? {
        None => d.heartbeat_secs,
        Some(n) => u32::try_from(n).map_err(|_| s.invalid("heartbeat_secs", "is too large"))?,
    };
    let reset_seq = s.optional_bool("reset_seq")?.unwrap_or(d.reset_seq_on_logon);
    let reconnect_secs = match s.optional_u64("reconnect_secs")? {
        None => d.reconnect_ns / 1_000_000_000,
        Some(0) => return Err(s.invalid("reconnect_secs", "must be at least 1")),
        Some(n) => n,
    };
    let logon_timeout_secs = match s.optional_u64("logon_timeout_secs")? {
        None => d.logon_timeout_ns / 1_000_000_000,
        Some(0) => return Err(s.invalid("logon_timeout_secs", "must be at least 1")),
        Some(n) => n,
    };
    let burst = match s.optional_u64("burst")? {
        None => d.burst,
        Some(0) => return Err(s.invalid("burst", "must be at least 1")),
        Some(n) => u32::try_from(n).map_err(|_| s.invalid("burst", "is too large"))?,
    };

    s.finish(FEED_KEYS)?;
    Ok(FixFeedConf {
        name,
        venue,
        mode,
        cores,
        ring,
        ring_slots,
        host,
        port,
        sender_comp_id,
        target_comp_id,
        begin_string,
        price_decimals,
        qty_decimals,
        default_qty,
        symbols,
        depth,
        subscription,
        heartbeat_secs,
        reset_seq,
        reconnect_secs,
        logon_timeout_secs,
        burst,
    })
}

/// What might be wrong with a conf that will still run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// The SMT layout could not be read, so the sibling rule was not applied.
    NoTopology,

    /// One symbol under two feeds on the same venue: two sessions, two rings,
    /// one instrument.
    SymbolInTwoFeeds {
        /// The venue.
        venue: Venue,
        /// The symbol.
        symbol: String,
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
            Self::SymbolInTwoFeeds { venue, symbol, a, b } => write!(
                f,
                "{} {symbol} is listed under both \"{a}\" and \"{b}\"; it will be published to two rings",
                venue.as_str()
            ),
        }
    }
}
