//! What a router is built from, and what it hands the binary back.
//!
//! ```text
//! conf ─→ Subscription { Instrument, channels, depth } ─→ Router::new
//!                                                            │
//!         subscriptions() · rest_books() · ticket()  ←───────┘   venue conversation, spelled out
//! ```
//!
//! ## One vocabulary for eight venues
//!
//! Every venue names its streams differently — `@depth@100ms`, `books`,
//! `orderbook.50.BTCUSDT`, `/market/level2:BTC-USDT` — and a conf that
//! repeated each spelling would be a conf that had to know all of them. So
//! the conf speaks in four words, [`Channel`], and the router translates. The
//! table is the venue's, not the conf's:
//!
//! | | `trade` | `bbo` | `book` | `delta` |
//! |---|---|---|---|---|
//! | binance | `@trade` / `@aggTrade` | `@bookTicker` | `@depth{N}@100ms` | `@depth@100ms` |
//! | upbit · bithumb | `trade` | — | `orderbook` | — |
//! | okx | `trades` | — | `books5` | `books` |
//! | bybit | `publicTrade` | — | `orderbook.{N}` | — |
//! | bitget | `trade` | — | `books` / `books{N}` | — |
//! | gate | `spot.trades` | — | — (REST) | `spot.order_book_update` |
//! | kucoin | `/market/match` · `/contractMarket/execution` | — | — (REST) | `/market/level2` · `/contractMarket/level2` |
//!
//! A word a venue has no stream for is refused when the router is built
//! ([`RouterError::Unsupported`]), which is what `--check` reports.
//!
//! `book` is *the venue's book channel, whatever it produces*: on Binance and
//! OKX that is a whole book every time, on Bybit and Bitget a whole book then
//! diffs — the decoder decides per frame. `delta` names a channel that is
//! diffs **only**, whose start book comes from somewhere else: the `book`
//! channel on Binance and OKX, and a REST body on Gate and KuCoin, which is
//! what [`RestBook`] is for.
//!
//! ## What a router is not given
//!
//! A socket, a clock it did not ask for, or an HTTP client. Every method on
//! [`Router`](crate::recv::pipeline::Router) that touches the outside world returns *what to
//! send* or *what to fetch*, and the binary does it — the same seam as
//! `jeed_fix::recv::Pipeline` handing back a `Reply` for the loop to write.

use crate::error::CryptoError;
use crate::instrument::Instrument;
use core::fmt;
use jeed_wire::{RecordSink, Venue, WireRecord};

/// A kind of stream, in the conf's vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    /// Prints.
    Trade,

    /// Best bid and ask, one level.
    Bbo,

    /// The venue's order-book channel.
    Book,

    /// A diffs-only book channel.
    Delta,
}

impl Channel {
    /// Every channel, in a fixed order.
    pub const ALL: [Self; 4] = [Self::Trade, Self::Bbo, Self::Book, Self::Delta];

    /// The conf spelling.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trade => "trade",
            Self::Bbo => "bbo",
            Self::Book => "book",
            Self::Delta => "delta",
        }
    }

    /// From the conf spelling.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == s)
    }

    #[inline]
    const fn bit(self) -> u8 {
        match self {
            Self::Trade => 1 << 0,
            Self::Bbo => 1 << 1,
            Self::Book => 1 << 2,
            Self::Delta => 1 << 3,
        }
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A set of [`Channel`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ChannelSet(u8);

impl ChannelSet {
    /// No channels.
    pub const EMPTY: Self = Self(0);

    /// `self` plus `channel`.
    #[inline]
    #[must_use]
    pub const fn with(self, channel: Channel) -> Self {
        Self(self.0 | channel.bit())
    }

    /// `true` if `channel` is in the set.
    #[inline]
    pub const fn contains(self, channel: Channel) -> bool {
        self.0 & channel.bit() != 0
    }

    /// `true` if nothing is in the set.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The channels in the set, in [`Channel::ALL`] order.
    pub fn iter(self) -> impl Iterator<Item = Channel> {
        Channel::ALL.into_iter().filter(move |&c| self.contains(c))
    }
}

impl FromIterator<Channel> for ChannelSet {
    fn from_iter<I: IntoIterator<Item = Channel>>(iter: I) -> Self {
        iter.into_iter().fold(Self::EMPTY, Self::with)
    }
}

impl fmt::Display for ChannelSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for c in self.iter() {
            if !first {
                f.write_str(",")?;
            }
            first = false;
            f.write_str(c.as_str())?;
        }
        Ok(())
    }
}

/// One instrument and the channels wanted for it.
#[derive(Debug, Clone, PartialEq)]
pub struct Subscription {
    /// The instrument, with its scales.
    pub instrument: Instrument,

    /// Which streams to ask for.
    pub channels: ChannelSet,

    /// Book depth where the venue offers a choice, in the venue's own
    /// units. `None` takes the router's default.
    pub depth: Option<u16>,
}

impl Subscription {
    /// Every channel in `channels`, default depth.
    pub fn new(instrument: Instrument, channels: ChannelSet) -> Self {
        Self { instrument, channels, depth: None }
    }

    /// The same, at `depth`.
    pub fn with_depth(instrument: Instrument, channels: ChannelSet, depth: u16) -> Self {
        Self { instrument, channels, depth: Some(depth) }
    }
}

/// Finds the subscription whose instrument is `symbol`, spelled as the venue
/// spells it.
///
/// A linear scan: a feed holds a handful of instruments, and one `memcmp`
/// per instrument per message is under the noise of the decode that
/// follows. A feed that grows to hundreds gets a table then.
#[inline]
pub fn lookup<'a>(subs: &'a [Subscription], symbol: &[u8]) -> Option<(usize, &'a Subscription)> {
    subs.iter().enumerate().find(|(_, s)| s.instrument.matches(symbol))
}

/// Publishes every print a batch yields, one slot each, and returns how many.
///
/// `next` is a decoder's `decode_next`: `Some(Ok)` filled the record,
/// `Some(Err)` refused this print, `None` means the batch is spent. The
/// first refused print ends the batch with its error — the pipeline then
/// counts the frame as failed — and the prints before it stay published.
pub(crate) fn publish_batch<S: RecordSink>(
    sink: &mut S,
    mut next: impl FnMut(&mut WireRecord) -> Option<Result<(), CryptoError>>,
) -> Result<usize, CryptoError> {
    let mut n = 0;
    loop {
        // `Err` is how a slot is abandoned; which kind of `Err` is kept aside.
        let mut refused: Option<CryptoError> = None;
        let mut spent = false;
        let published = sink.publish(|rec| match next(rec) {
            Some(Ok(())) => Ok(()),
            Some(Err(e)) => {
                refused = Some(e);
                Err(e)
            }
            None => {
                spent = true;
                Err(CryptoError::Empty)
            }
        });
        match (published, refused, spent) {
            (Ok(()), _, _) => n += 1,
            (Err(_), Some(e), _) => return Err(e),
            (Err(_), None, _) => return Ok(n),
        }
    }
}

/// Checks what every router checks: at least one subscription, every
/// instrument on `venue`, every subscription with a channel the venue has,
/// no symbol twice.
///
/// `supported` is the venue's channel set; `depth_ok` says whether a
/// requested depth is one the venue offers.
pub(crate) fn check(
    venue: Venue,
    subs: &[Subscription],
    supported: ChannelSet,
    depth_ok: impl Fn(u16) -> bool,
) -> Result<(), RouterError> {
    if subs.is_empty() {
        return Err(RouterError::NoInstruments);
    }
    for (i, sub) in subs.iter().enumerate() {
        let inst = &sub.instrument;
        if inst.venue() != venue {
            return Err(RouterError::WrongVenue { expected: venue, found: inst.venue() });
        }
        let symbol = || String::from_utf8_lossy(inst.symbol_bytes()).into_owned();
        if sub.channels.is_empty() {
            return Err(RouterError::NoChannels { symbol: symbol() });
        }
        if subs[..i].iter().any(|s| s.instrument.matches(inst.symbol_bytes())) {
            return Err(RouterError::Duplicate { symbol: symbol() });
        }
        for channel in sub.channels.iter() {
            if !supported.contains(channel) {
                return Err(RouterError::Unsupported { venue, channel });
            }
        }
        if let Some(depth) = sub.depth
            && !depth_ok(depth)
        {
            return Err(RouterError::Depth { venue, depth });
        }
    }
    Ok(())
}

/// HTTP method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// `GET`.
    Get,

    /// `POST` with an empty body.
    Post,
}

impl Method {
    /// The request-line token.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

/// A REST call the binary makes on the router's behalf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    /// Method.
    pub method: Method,

    /// `https://host/path?query`, complete.
    pub url: String,
}

impl HttpRequest {
    /// A `GET`.
    pub fn get(url: impl Into<String>) -> Self {
        Self { method: Method::Get, url: url.into() }
    }

    /// A `POST` with no body.
    pub fn post(url: impl Into<String>) -> Self {
        Self { method: Method::Post, url: url.into() }
    }
}

impl fmt::Display for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.method.as_str(), self.url)
    }
}

/// A start book to fetch over REST after subscribing, and which instrument
/// it is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestBook {
    /// Index into the router's subscriptions.
    pub instrument: usize,

    /// The request.
    pub request: HttpRequest,
}

/// Why a router could not be built from its subscriptions, or could not
/// read a ticket.
///
/// Every variant but [`Ticket`](Self::Ticket) is a conf mistake, found
/// before a socket is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouterError {
    /// The venue is not a crypto venue this crate routes.
    NotCrypto(Venue),

    /// No subscriptions at all.
    NoInstruments,

    /// A subscription's instrument is on a different venue than the router.
    WrongVenue {
        /// The router's.
        expected: Venue,
        /// The instrument's.
        found: Venue,
    },

    /// A subscription with no channels.
    NoChannels {
        /// Its symbol.
        symbol: String,
    },

    /// Two subscriptions for one symbol.
    Duplicate {
        /// The symbol.
        symbol: String,
    },

    /// The venue has no stream for this channel.
    Unsupported {
        /// The venue.
        venue: Venue,
        /// The channel.
        channel: Channel,
    },

    /// A depth the venue does not offer.
    Depth {
        /// The venue.
        venue: Venue,
        /// What was asked.
        depth: u16,
    },

    /// A ticket body (KuCoin's `bullet-public`) could not be read.
    Ticket(&'static str),
}

impl fmt::Display for RouterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotCrypto(v) => write!(f, "{} is not a crypto venue", v.as_str()),
            Self::NoInstruments => write!(f, "no instruments"),
            Self::WrongVenue { expected, found } => {
                write!(f, "instrument is on {} but the feed is {}", found.as_str(), expected.as_str())
            }
            Self::NoChannels { symbol } => write!(f, "{symbol}: no channels"),
            Self::Duplicate { symbol } => write!(f, "{symbol} is listed twice"),
            Self::Unsupported { venue, channel } => {
                write!(f, "{} has no `{channel}` channel", venue.as_str())
            }
            Self::Depth { venue, depth } => write!(f, "{} has no book of depth {depth}", venue.as_str()),
            Self::Ticket(what) => write!(f, "ticket: {what}"),
        }
    }
}

impl core::error::Error for RouterError {}
