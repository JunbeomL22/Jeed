//! The FIX handler: conf → ring → one session per feed, and the venue adapter
//! that turns decoded market data into wire records.
//!
//! ```text
//!  main thread                         feed thread "smbs"
//!  ──────────────────────────────      ─────────────────────────────────────
//!  FixConf::load, validate             pin
//!  for each feed:                      loop {
//!    RingProducer::create   ─┐           poll_once  (connect · logon · frames)
//!    SnapshotAdapter        ─┼─ move ─→   logged on and freshly so? → 35=V
//!    Receiver::new          ─┘           report every 10 s
//!  spawn                               }
//!  wait for stop / a death
//! ```
//!
//! ## Same shape as crypto, one difference
//!
//! Rings are created and adapters built on the main thread in conf order, so a
//! conf that names a bad venue or an unopenable ring fails with nothing
//! running ([`crypto`](crate::crypto)). What FIX adds over a WebSocket feed is
//! a *session*: after every logon the binary sends one `35=V`
//! ([`Receiver::subscribe`](jeed_fix::recv::Receiver::subscribe)), because what
//! to request is venue conversation, not protocol obligation. Everything else
//! — heartbeats, resends, the silence check, reconnect — the receiver does on
//! its own between rounds; this thread only re-subscribes and reports.
//!
//! ## The adapter is here, not in `jeed-fix`
//!
//! `jeed-fix` knows FIX and no venue (`documents/feed_handler.md` §13), so the
//! step to a wire record is a trait it leaves unimplemented. This crate, which
//! already knows every venue, supplies [`SnapshotAdapter`]: the venue-agnostic
//! FIX 4.4 mapping, driven by conf, that refuses what it cannot map without
//! guessing. A venue with quirks of its own — a private incremental encoding,
//! an absent size that means something — specialises it beside that venue when
//! the session is confirmed live.

use crate::conf::fix::{FixConf, FixFeedConf};
use crate::cpu::{self, CpuError};
use crate::feed::Death;
use crate::{info, warn};

pub use crate::feed::Options;
use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};
use jeed_fix::clock;
use jeed_fix::recv::{
    Emitter, LinkState, Mode, Receiver, SubscriptionRequest, SubscriptionType, SymbolFilter,
};
use jeed_fix::{FixScales, MdEntryType, MdMessage};
use jeed_shm::{RingProducer, SegmentName, ShmError};
use jeed_wire::{
    QuotePayload, RecordHeader, RecordSink, Symbol, TradePayload, UnixNano, Venue, WireKind,
    WireLevel, WireRecord, header_flags, symbol_from_bytes,
};
use std::sync::Arc;

/// A running FIX feed.
pub type Feed = crate::feed::Feed<FeedError>;

/// The entry types every subscription asks for: both book sides and the trade
/// tape. A venue that does not honour one simply never sends it.
const ENTRY_TYPES: &[MdEntryType] = &[MdEntryType::Bid, MdEntryType::Offer, MdEntryType::Trade];

// ===========================================================================
// The adapter
// ===========================================================================

/// The venue layer, driven by conf: a full refresh becomes a book snapshot, a
/// trade becomes a trade, and anything ambiguous is refused.
///
/// Everything venue-specific here is a conf fact — the [`Venue`] byte, the
/// price and size [`FixScales`], and the size to use when a snapshot level
/// carries none. What is *not* configurable is the refusal: a `35=X` that
/// moves the book has no wire record to map onto ([`WireKind::SnapshotDelta`]
/// is reserved for a venue whose delta encoding is understood), and a level
/// with no size and no configured default cannot be published as a number that
/// was never sent. Both come back as an error the loop counts, never as a
/// wrong book (`documents/feed_handler.md` §8, §13).
#[derive(Debug, Clone, Copy)]
pub struct SnapshotAdapter {
    venue: Venue,
    scales: FixScales,
    default_qty: Option<u64>,
}

impl SnapshotAdapter {
    /// Builds the adapter for a feed.
    pub fn new(venue: Venue, scales: FixScales, default_qty: Option<u64>) -> Self {
        Self { venue, scales, default_qty }
    }

    /// The size to publish for an entry, or an error when it has none and no
    /// default is configured.
    #[inline]
    fn size_of(&self, entry: &jeed_fix::MdEntry) -> Result<u64, AdaptError> {
        if entry.has_size {
            Ok(entry.size)
        } else {
            self.default_qty.ok_or(AdaptError::NoSize)
        }
    }

    fn snapshot<S: RecordSink>(
        &self,
        msg: &MdMessage,
        recv_ns: UnixNano,
        sink: &mut S,
    ) -> Result<usize, AdaptError> {
        let symbol = wire_symbol(msg.symbol.as_bytes())?;
        let mut quote = QuotePayload::default();
        let mut depth = 0usize;
        for entry in msg.entries() {
            let side = match entry.entry_type {
                MdEntryType::Bid | MdEntryType::Offer => entry.entry_type,
                // A statistic in a snapshot — session high, VWAP — is not a
                // book level. The decoder keeps it so a venue adding one cannot
                // break the parse; the adapter passes over it.
                _ => continue,
            };
            let level = entry.level as usize;
            if level >= jeed_wire::WIRE_MAX_DEPTH {
                continue;
            }
            let wire = WireLevel::new(entry.price, self.size_of(entry)?);
            if matches!(side, MdEntryType::Bid) {
                quote.set_bid(level, wire);
            } else {
                quote.set_ask(level, wire);
            }
            depth = depth.max(level + 1);
        }

        let mut header = RecordHeader::new(WireKind::Quote, self.venue, symbol, recv_ns);
        header.set_scales(self.scales.price, self.scales.quantity);
        header.set_depth(depth as u8);
        if let Some(venue_ns) = msg.venue_time() {
            header.set_venue_time(venue_ns);
        }
        if msg.side_depth(MdEntryType::Bid) == 0 {
            header.set_flags(header_flags::BID_EMPTY);
        }
        if msg.side_depth(MdEntryType::Offer) == 0 {
            header.set_flags(header_flags::ASK_EMPTY);
        }

        sink.publish(|rec| {
            *rec = WireRecord::new_quote(header, quote);
            Ok::<(), AdaptError>(())
        })?;
        Ok(1)
    }

    fn incremental<S: RecordSink>(
        &self,
        msg: &MdMessage,
        recv_ns: UnixNano,
        sink: &mut S,
    ) -> Result<usize, AdaptError> {
        let mut published = 0;
        for entry in msg.entries() {
            match entry.entry_type {
                MdEntryType::Trade => {}
                // A book move with no delta record to carry it: refuse, so the
                // consumer resynchronises rather than reads a book missing the
                // levels this message changed (§8).
                MdEntryType::Bid | MdEntryType::Offer => return Err(AdaptError::IncrementalBook),
                _ => continue,
            }
            let symbol = wire_symbol(msg.entry_symbol(entry))?;
            let payload = TradePayload::new(entry.price, self.size_of(entry)?);
            let mut header = RecordHeader::new(WireKind::Trade, self.venue, symbol, recv_ns);
            header.set_scales(self.scales.price, self.scales.quantity);
            if let Some(venue_ns) = entry.venue_time() {
                header.set_venue_time(venue_ns);
            }
            sink.publish(|rec| {
                *rec = WireRecord::new_trade(header, payload);
                Ok::<(), AdaptError>(())
            })?;
            published += 1;
        }
        Ok(published)
    }
}

impl jeed_fix::recv::MdAdapter for SnapshotAdapter {
    type Error = AdaptError;

    fn venue(&self) -> Venue {
        self.venue
    }

    fn scales(&self) -> FixScales {
        self.scales
    }

    fn adapt<S: RecordSink>(
        &mut self,
        msg: &MdMessage,
        recv_ns: UnixNano,
        sink: &mut S,
    ) -> Result<usize, AdaptError> {
        if msg.is_snapshot() {
            self.snapshot(msg, recv_ns, sink)
        } else {
            self.incremental(msg, recv_ns, sink)
        }
    }
}

/// The venue's symbol as the header carries it — its own name for the
/// instrument, `NUL`-padded, or an error when it is missing or too long.
fn wire_symbol(symbol: &[u8]) -> Result<Symbol, AdaptError> {
    if symbol.is_empty() {
        return Err(AdaptError::NoSymbol);
    }
    symbol_from_bytes(symbol).ok_or(AdaptError::SymbolTooLong)
}

/// Why the adapter refused a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdaptError {
    /// A `35=X` entry that moves the book, which the wire has no delta record
    /// for yet (`documents/feed_handler.md` §8).
    IncrementalBook,

    /// A level with no size and no `default_qty` in conf: publishing a size
    /// the venue never sent would be inventing a number.
    NoSize,

    /// No `55` to key the record on.
    NoSymbol,

    /// A `55` longer than the header's symbol field.
    SymbolTooLong,
}

impl fmt::Display for AdaptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IncrementalBook => write!(f, "35=X moves the book and the wire has no delta record"),
            Self::NoSize => write!(f, "entry has no size and no default_qty is configured"),
            Self::NoSymbol => write!(f, "no Symbol to key the record on"),
            Self::SymbolTooLong => {
                write!(f, "Symbol is longer than {} bytes", jeed_wire::SYMBOL_LEN)
            }
        }
    }
}

impl std::error::Error for AdaptError {}

// ===========================================================================
// The wiring
// ===========================================================================

type Rx = Receiver<SnapshotAdapter, RingProducer>;

/// What a blocking feed sleeps per round while disconnected — matched to the
/// read timeout so a reconnecting feed does not spin a core.
const DISCONNECTED_TICK: std::time::Duration = std::time::Duration::from_millis(50);

/// Creates every feed's ring and adapter and starts its thread.
///
/// Nothing is dialled here — a venue down at start is one the thread
/// reconnects to. Nothing is started if anything fails.
pub fn start(conf: &FixConf, opts: Options, stop: Arc<AtomicBool>) -> Result<Vec<Feed>, StartError> {
    let mut ready = Vec::with_capacity(conf.feeds.len());

    for feed in &conf.feeds {
        let name = SegmentName::local(&feed.ring)
            .map_err(|source| StartError::Ring { feed: feed.name.clone(), source })?;
        let ring = RingProducer::create(&name, feed.ring_slots, opts.boot_id)
            .map_err(|source| StartError::Ring { feed: feed.name.clone(), source })?;
        if ring.reused_existing_section() {
            warn!(
                "{}: ring {} already existed (a consumer is holding it, or the last run did not exit); \
                 header rewritten, {} drops carried over",
                feed.name, feed.ring, ring.drops()
            );
        }
        info!(
            "{}: ring {} · {} slots · {} KiB · boot {:#018x}",
            feed.name,
            feed.ring,
            feed.ring_slots,
            (feed.ring_slots * jeed_wire::WIRE_RECORD_LEN as u64) >> 10,
            opts.boot_id
        );

        let adapter = SnapshotAdapter::new(feed.venue, feed.scales(), feed.default_qty);
        let emitter = Emitter::new(feed.sender_comp_id.as_bytes(), feed.target_comp_id.as_bytes())
            .with_begin_string(feed.begin_string.as_bytes());
        let symbols = SymbolFilter::new(feed.symbols.iter().map(String::as_bytes));
        let rx = Receiver::new(feed.receiver_config(conf.health), feed.endpoint(), emitter, symbols, adapter, ring);
        ready.push((feed, rx));
    }

    let mut feeds = Vec::with_capacity(ready.len());
    for (feed, rx) in ready {
        let name = feed.name.clone();
        let stop = Arc::clone(&stop);
        let conf = feed.clone();
        let handle = std::thread::Builder::new()
            .name(name.clone())
            .spawn(move || run(&conf, opts, rx, &stop))
            .map_err(|source| StartError::Spawn { feed: name.clone(), source })?;
        feeds.push(Feed::new(name, handle));
    }
    Ok(feeds)
}

/// The feed thread.
fn run(feed: &FixFeedConf, opts: Options, mut rx: Rx, stop: &AtomicBool) -> Result<(), FeedError> {
    let name = &feed.name;
    if opts.pin {
        let mask = feed.mask().ok_or(CpuError::BeyondMask)?;
        cpu::pin_current_thread(mask)?;
        info!("{name}: pinned to {:?} · now on processor {}", feed.cores, cpu::current_processor());
    } else {
        info!("{name}: not pinned (--no-pin)");
    }
    let cfg = *rx.config();
    info!(
        "{name}: {} · {}→{} · burst {} · heartbeat {} s · reconnect {} s · logon timeout {} s · \
         record heartbeat {} ms · stale {} ms",
        match cfg.mode { Mode::Spin => "spin", Mode::Block => "block" },
        feed.sender_comp_id,
        feed.target_comp_id,
        cfg.burst,
        cfg.heartbeat_secs,
        cfg.reconnect_ns / 1_000_000_000,
        cfg.logon_timeout_ns / 1_000_000_000,
        cfg.record_heartbeat_ns / 1_000_000,
        cfg.stale_ns / 1_000_000,
    );
    info!("{name}: dialling {}", rx.endpoint());
    info!(
        "{name}: will request {} symbol(s) at depth {} · {}",
        feed.symbols.len(),
        feed.depth,
        match feed.subscription {
            SubscriptionType::Snapshot => "snapshot once",
            SubscriptionType::SnapshotPlusUpdates => "snapshot then updates",
            SubscriptionType::Unsubscribe => "unsubscribe",
        }
    );

    let spin = cfg.mode == Mode::Spin;
    let mut logons_seen = 0u64;
    let mut rounds: u32 = 0;
    let mut next_report = clock::now_ns().saturating_add(opts.report_ns);

    while !stop.load(Ordering::Relaxed) {
        if let Some(e) = rx.poll_once() {
            if e.is_closed() {
                info!("{name}: {e}");
            } else {
                warn!("{name}: {e}");
            }
        }

        // A fresh logon — the first, or one after a reconnect — is when the
        // subscription must be (re-)sent: the venue forgot it on the drop.
        let logons = rx.stats().logons;
        if logons > logons_seen && rx.state() == LinkState::LoggedOn {
            logons_seen = logons;
            subscribe(name, feed, &mut rx);
        }

        if !spin && rx.state() != LinkState::LoggedOn {
            std::thread::sleep(DISCONNECTED_TICK);
        }
        rounds = rounds.wrapping_add(1);
        if opts.report_ns != 0 && (!spin || rounds & 0xFF == 0) {
            let now = clock::now_ns();
            if now >= next_report {
                report(name, &rx);
                next_report = now.saturating_add(opts.report_ns);
            }
        }
    }

    info!("{name}: stopping");
    rx.shutdown("handler stopping");
    report(name, &rx);
    Ok(())
}

/// Sends the one `35=V` this feed's conf describes.
fn subscribe(name: &str, feed: &FixFeedConf, rx: &mut Rx) {
    let symbols: Vec<&[u8]> = feed.symbols.iter().map(String::as_bytes).collect();
    let req = SubscriptionRequest {
        req_id: name.as_bytes(),
        subscription: feed.subscription,
        depth: feed.depth,
        update_type: None,
        entry_types: ENTRY_TYPES,
        symbols: &symbols,
    };
    match rx.subscribe(&req) {
        Ok(()) => info!("{name}: subscribed to {} symbol(s)", symbols.len()),
        Err(e) => warn!("{name}: subscribe: {e}"),
    }
}

/// One line of counters.
fn report(name: &str, rx: &Rx) {
    let s = rx.stats();
    let ring = rx.pipeline().sink();
    info!(
        "{name}: {} · msg {} · md {} · pub {} · hb {} · stale {} · fail {} · refused {} · \
         admin {} · filtered {} · gaps {} (lost {} dup {}) · logons {} · disc {} · connfail {} · \
         framing {} · ring seq {} drops {}",
        match rx.state() {
            LinkState::LoggedOn => "logged-on",
            LinkState::LoggingOn => "logging-on",
            LinkState::Disconnected => "disconnected",
        },
        s.received,
        s.market_data,
        s.published,
        s.heartbeats,
        s.stale,
        s.decode_failed,
        s.refused,
        s.admin,
        s.filtered_symbol,
        s.gaps,
        s.lost,
        s.duplicates,
        s.logons,
        s.disconnects,
        s.connect_failures,
        s.framing_errors,
        ring.next_seq(),
        ring.drops(),
    );
}

/// Why a feed thread ended early.
#[derive(Debug)]
pub enum FeedError {
    /// Could not pin to the configured cores.
    Pin(CpuError),

    /// The receive loop panicked.
    Panicked,
}

impl fmt::Display for FeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pin(e) => write!(f, "could not pin: {e}"),
            Self::Panicked => write!(f, "receive loop panicked"),
        }
    }
}

impl std::error::Error for FeedError {}

impl Death for FeedError {
    fn panicked() -> Self {
        Self::Panicked
    }
}

impl From<CpuError> for FeedError {
    fn from(e: CpuError) -> Self {
        Self::Pin(e)
    }
}

/// Why the handler could not start.
#[derive(Debug)]
pub enum StartError {
    /// A ring could not be created.
    Ring {
        /// Feed.
        feed: String,
        /// Why not.
        source: ShmError,
    },

    /// A feed thread could not be spawned.
    Spawn {
        /// Feed.
        feed: String,
        /// Why not.
        source: std::io::Error,
    },
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ring { feed, source } => write!(f, "{feed}: ring: {source}"),
            Self::Spawn { feed, source } => write!(f, "{feed}: spawn: {source}"),
        }
    }
}

impl std::error::Error for StartError {}
