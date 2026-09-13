//! `conf/crypto.toml`.
//!
//! ```toml
//! [[feed]]
//! name = "binance-spot"   venue = "binance-spot"   mode = "block"   cores = [4]
//! ring = "jeed.crypto.binance-spot"   ring_slots = 16384
//! # url = "wss://…"   rest = "https://…"   ping_secs = 30   reconnect_secs = 5   burst = 64
//!
//! [[feed.instrument]]
//! symbol = "BTCUSDT"   price_decimals = 2   qty_decimals = 5
//! channels = ["trade", "bbo", "book", "delta"]
//! # depth = 10
//!
//! [health]  heartbeat_ms = 100   stale_ms = 500
//! [log]     report_secs = 10
//! ```
//!
//! ## One feed = one connection = one ring
//!
//! A `[[feed]]` is one WebSocket to one venue, and everything on it goes to
//! one ring. The venue picks the router
//! ([`VenueRouter`]); the instruments under
//! it are what that connection subscribes to. Two venues are two feeds, and
//! so are two connections to one venue.
//!
//! ## What is checked
//!
//! The placement rules every conf shares ([`rules`](super::rules)), then
//! per feed: at least one instrument, each instrument's symbol and scales
//! fit the wire, and the venue's router accepts the channels and depth asked
//! for — a `book` channel on Gate, a depth of seven on Bybit, are refused
//! here rather than met as a subscription error at 09:00. The check is the
//! router's own constructor, so `--check` and the start agree.
//!
//! ## Scales are configuration
//!
//! `price_decimals` and `qty_decimals` come from the venue's instrument
//! reference (`exchangeInfo` and its equivalents), not from a message. A
//! wrong scale is caught per frame — the decoders refuse a digit the scale
//! cannot keep — but only a conf can say what the right one is (`CLAUDE.md`).

use crate::conf::rules::{Placement, RuleError, check_placement};
use crate::conf::{ConfError, Health, Section, read};
use crate::cpu::{Topology, mask_of};
use crate::toml::Table;
use core::fmt;
use jeed_crypto::Instrument;
use jeed_crypto::recv::{Channel, ChannelSet, Config, Endpoint, LinkOptions, Mode, Subscription, VenueRouter};
use jeed_wire::Venue;
use std::path::Path;

/// The whole file.
#[derive(Debug, Clone, PartialEq)]
pub struct CryptoConf {
    /// `[[feed]]`, in file order.
    pub feeds: Vec<CryptoFeedConf>,

    /// `[health]`.
    pub health: Health,

    /// `[log] report_secs` — cadence of the per-feed report line. Zero
    /// silences it.
    pub report_secs: u64,
}

/// One `[[feed]]`: one connection, one thread, one ring.
#[derive(Debug, Clone, PartialEq)]
pub struct CryptoFeedConf {
    /// `name` — for the thread and the log.
    pub name: String,

    /// `venue` — which exchange and market, by its conf name
    /// ([`venue_by_name`]).
    pub venue: Venue,

    /// `mode` — `"spin"` or `"block"`.
    pub mode: Mode,

    /// `cores` — the affinity set. Exactly one for a spinning feed.
    pub cores: Vec<u16>,

    /// `ring` — segment name, session-local namespace.
    pub ring: String,

    /// `ring_slots` — a power of two.
    pub ring_slots: u64,

    /// `url` — the WebSocket to dial, if not the venue's default. Required
    /// for nothing; KuCoin's comes from a ticket either way.
    pub url: Option<Endpoint>,

    /// `rest` — where the venue's REST calls go, if not the venue's own
    /// (`https://host[:port]`). Gate and KuCoin only.
    pub rest: Option<String>,

    /// `[[feed.instrument]]`, in file order.
    pub instruments: Vec<InstrumentConf>,

    /// `ping_secs` — quiet time before a protocol ping; twice it with nothing
    /// back and the connection is dropped. Zero disables it.
    pub ping_secs: u64,

    /// `reconnect_secs` — wait between connection attempts.
    pub reconnect_secs: u64,

    /// `burst` — frames handled per round.
    pub burst: u32,
}

/// One `[[feed.instrument]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstrumentConf {
    /// `symbol` — as the venue spells it.
    pub symbol: String,

    /// `price_decimals`.
    pub price_decimals: usize,

    /// `qty_decimals`.
    pub qty_decimals: usize,

    /// `channels` — in the conf vocabulary.
    pub channels: ChannelSet,

    /// `depth` — book depth, where the venue offers a choice.
    pub depth: Option<u16>,
}

impl CryptoFeedConf {
    /// The receiver's configuration for this feed under `health`.
    pub fn receiver_config(&self, health: Health) -> Config {
        Config {
            mode: self.mode,
            ping_interval_ns: self.ping_secs * 1_000_000_000,
            record_heartbeat_ns: health.heartbeat_ns,
            stale_ns: health.stale_ns,
            burst: self.burst,
            reconnect_ns: self.reconnect_secs * 1_000_000_000,
            link: LinkOptions::default(),
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

    /// The instruments as the router takes them.
    pub fn subscriptions(&self) -> Result<Vec<Subscription>, RuleError> {
        self.instruments
            .iter()
            .map(|i| {
                let inst = Instrument::new(self.venue, i.symbol.as_bytes(), i.price_decimals, i.qty_decimals)
                    .map_err(|source| RuleError::Instrument {
                        feed: self.name.clone(),
                        symbol: i.symbol.clone(),
                        source,
                    })?;
                Ok(Subscription { instrument: inst, channels: i.channels, depth: i.depth })
            })
            .collect()
    }

    /// The venue's router over this feed's instruments, with `rest` applied.
    pub fn router(&self) -> Result<VenueRouter, RuleError> {
        if self.instruments.is_empty() {
            return Err(RuleError::NoInstruments { feed: self.name.clone() });
        }
        let router = VenueRouter::new(self.venue, self.subscriptions()?)
            .map_err(|source| RuleError::Router { feed: self.name.clone(), source })?;
        Ok(match &self.rest {
            Some(base) => router.with_rest(base),
            None => router,
        })
    }

    /// Where to dial: `url`, or the venue's default. `None` for a venue
    /// whose address comes from a ticket.
    pub fn endpoint(&self) -> Option<Endpoint> {
        self.url.clone().or_else(|| VenueRouter::default_url(self.venue).map(|u| u.parse().expect("a known URL")))
    }
}

const FEED_KEYS: &[&str] = &[
    "name",
    "venue",
    "mode",
    "cores",
    "ring",
    "ring_slots",
    "url",
    "rest",
    "instrument",
    "ping_secs",
    "reconnect_secs",
    "burst",
];
const INSTRUMENT_KEYS: &[&str] = &["symbol", "price_decimals", "qty_decimals", "channels", "depth"];
const ROOT_KEYS: &[&str] = &["feed", "health", "log"];

/// Conf names and their venues.
const VENUES: &[(&str, Venue)] = &[
    ("binance-spot", Venue::BinanceSpot),
    ("binance-futures", Venue::BinanceFutures),
    ("upbit", Venue::Upbit),
    ("bithumb", Venue::Bithumb),
    ("okx", Venue::Okx),
    ("bybit-spot", Venue::BybitSpot),
    ("bybit-linear", Venue::BybitLinear),
    ("bitget-spot", Venue::BitgetSpot),
    ("bitget-linear", Venue::BitgetLinear),
    ("gate-spot", Venue::GateSpot),
    ("kucoin-spot", Venue::KucoinSpot),
    ("kucoin-futures", Venue::KucoinFutures),
];

/// The venue a conf `venue = "…"` names.
pub fn venue_by_name(name: &str) -> Option<Venue> {
    VENUES.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
}

/// The conf name of a crypto venue, or `None` for one that is not.
pub fn venue_name(venue: Venue) -> Option<&'static str> {
    VENUES.iter().find(|(_, v)| *v == venue).map(|(n, _)| *n)
}

/// Every conf venue name, for the error message.
pub fn venue_names() -> impl Iterator<Item = &'static str> {
    VENUES.iter().map(|(n, _)| *n)
}

impl CryptoConf {
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
    ///
    /// `topology` is the box's SMT layout; without it the sibling rule is
    /// skipped (and [`warnings`](Self::warnings) says so).
    pub fn validate(&self, topology: Option<&Topology>) -> Result<(), RuleError> {
        let placements: Vec<Placement<'_>> = self.feeds.iter().map(CryptoFeedConf::placement).collect();
        check_placement(&placements, topology)?;
        for feed in &self.feeds {
            feed.router()?;
        }
        Ok(())
    }

    /// What might be wrong.
    pub fn warnings(&self, topology: Option<&Topology>) -> Vec<Warning> {
        let mut out = Vec::new();
        if topology.is_none() {
            out.push(Warning::NoTopology);
        }
        for (i, feed) in self.feeds.iter().enumerate() {
            for inst in &feed.instruments {
                if let Some(other) = self.feeds[..i]
                    .iter()
                    .find(|f| f.venue == feed.venue && f.instruments.iter().any(|o| o.symbol == inst.symbol))
                {
                    out.push(Warning::SymbolInTwoFeeds {
                        venue: feed.venue,
                        symbol: inst.symbol.clone(),
                        a: other.name.clone(),
                        b: feed.name.clone(),
                    });
                }
            }
        }
        out
    }
}

fn feed(s: &Section<'_>, name: String) -> Result<CryptoFeedConf, ConfError> {
    let venue_text = s.required_str("venue")?;
    let venue = venue_by_name(venue_text).ok_or_else(|| {
        let known: Vec<&str> = venue_names().collect();
        s.invalid("venue", format!("\"{venue_text}\" is not a venue; one of {}", known.join(", ")))
    })?;
    let mode = match s.required_str("mode")? {
        "spin" => Mode::Spin,
        "block" => Mode::Block,
        other => return Err(s.invalid("mode", format!("must be \"spin\" or \"block\", found \"{other}\""))),
    };
    let cores = s.required_u16s("cores")?;
    let ring = s.required_str("ring")?.to_owned();
    let ring_slots = s.required_u64("ring_slots")?;

    let url = match s.optional_str("url")? {
        None => None,
        Some(text) => Some(text.parse::<Endpoint>().map_err(|e| s.invalid("url", format!("`{text}`: {e}")))?),
    };
    let rest = match s.optional_str("rest")? {
        None => None,
        Some(text) if text.starts_with("https://") || text.starts_with("http://") => Some(text.to_owned()),
        Some(text) => return Err(s.invalid("rest", format!("`{text}` must start with http:// or https://"))),
    };

    let mut instruments = Vec::new();
    for (i, t) in s.tables("instrument")?.into_iter().enumerate() {
        let at = format!("{}.instrument[{i}]", s.at);
        let section = Section::new(&at, t);
        let symbol = section.required_str("symbol")?.to_owned();
        let at = format!("{}.instrument[{symbol}]", s.at);
        let section = Section::new(&at, t);
        instruments.push(instrument(&section, symbol)?);
    }

    let d = Config::default();
    let ping_secs = s.optional_u64("ping_secs")?.unwrap_or(d.ping_interval_ns / 1_000_000_000);
    let reconnect_secs = match s.optional_u64("reconnect_secs")? {
        None => d.reconnect_ns / 1_000_000_000,
        Some(0) => return Err(s.invalid("reconnect_secs", "must be at least 1")),
        Some(n) => n,
    };
    let burst = match s.optional_u64("burst")? {
        None => d.burst,
        Some(0) => return Err(s.invalid("burst", "must be at least 1")),
        Some(n) => u32::try_from(n).map_err(|_| s.invalid("burst", "is too large"))?,
    };

    s.finish(FEED_KEYS)?;
    Ok(CryptoFeedConf {
        name,
        venue,
        mode,
        cores,
        ring,
        ring_slots,
        url,
        rest,
        instruments,
        ping_secs,
        reconnect_secs,
        burst,
    })
}

fn instrument(s: &Section<'_>, symbol: String) -> Result<InstrumentConf, ConfError> {
    let decimals = |key: &'static str| -> Result<usize, ConfError> {
        let n = s.required_u64(key)?;
        usize::try_from(n).ok().filter(|&n| n <= 8).ok_or_else(|| s.invalid(key, "must be 0..=8"))
    };
    let price_decimals = decimals("price_decimals")?;
    let qty_decimals = decimals("qty_decimals")?;

    let mut channels = ChannelSet::EMPTY;
    for text in s.required_strs("channels")? {
        let channel = Channel::parse(text).ok_or_else(|| {
            let known: Vec<&str> = Channel::ALL.iter().map(|c| c.as_str()).collect();
            s.invalid("channels", format!("`{text}` is not a channel; one of {}", known.join(", ")))
        })?;
        if channels.contains(channel) {
            return Err(s.invalid("channels", format!("`{text}` is listed twice")));
        }
        channels = channels.with(channel);
    }
    if channels.is_empty() {
        return Err(s.invalid("channels", "is empty"));
    }

    let depth = match s.optional_u64("depth")? {
        None => None,
        Some(0) => return Err(s.invalid("depth", "must be at least 1")),
        Some(n) => Some(u16::try_from(n).map_err(|_| s.invalid("depth", "is too large"))?),
    };

    s.finish(INSTRUMENT_KEYS)?;
    Ok(InstrumentConf { symbol, price_decimals, qty_decimals, channels, depth })
}

/// What might be wrong with a conf that will still run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// The SMT layout could not be read, so the sibling rule was not applied.
    NoTopology,

    /// One instrument under two feeds on the same venue: two connections,
    /// two rings, one market.
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
