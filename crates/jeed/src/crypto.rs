//! The crypto handler: conf → rings → routers → one thread per connection.
//!
//! ```text
//!  main thread                              feed thread "binance-spot"
//!  ─────────────────────────────            ─────────────────────────────────────
//!  CryptoConf::load, validate               pin
//!  for each feed:                           loop {
//!    RingProducer::create   ─┐                ticket due?  → POST, set_endpoint
//!    VenueRouter::new       ─┼─ move ─→        poll_once   (connect · frames · pings)
//!    Receiver::new          ─┘                opened?     → send subscriptions,
//!  spawn                                                    GET start books → ring
//!  wait for stop / a death                    report every 10 s
//!                                           }
//! ```
//!
//! ## Same shape as KRX, one difference
//!
//! Rings are created and routers built on the main thread in conf order, so
//! a conf that names a channel Gate does not have fails with nothing
//! running ([`krx`](crate::krx)). What KRX does not have is *conversation*:
//! a WebSocket feed must subscribe after every open, some venues want a REST
//! start book after that, and KuCoin will not even say where its socket is
//! until asked. All three are venue knowledge, and all three live in the
//! router ([`Router`]); this thread only does what
//! the router says and does it **between rounds** — never inside the receive
//! loop, which stays one socket and no blocking call.
//!
//! ## The REST fetch blocks the feed thread
//!
//! For the round-trip of one HTTP request, once per open. Frames that arrive
//! meanwhile wait in the socket buffer, which is what the venue's own
//! resynchronisation procedure asks for anyway ("subscribe, buffer, fetch,
//! apply"). A spinning feed on a pinned core stalls its ring's heartbeat for
//! that long; the consumer's stale guard is the tolerance for it.
//!
//! ## Disconnected, a blocking feed sleeps
//!
//! `poll_once` returns at once while the reconnect wait runs. A spinning
//! feed spins through it — the core is its anyway — but a blocking feed
//! would burn a core waiting for a venue to come back, so it sleeps a tick.

use crate::conf::{CryptoConf, CryptoFeedConf};
use crate::cpu::{self, CpuError};
use crate::feed::Death;
use crate::{info, warn};

pub use crate::feed::Options;
use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};
use jeed_crypto::clock;
use jeed_crypto::recv::http::{self, HttpError};
use jeed_crypto::recv::{Endpoint, LinkState, Mode, Outcome, Receiver, Router, RouterError, Tls, VenueRouter};
use jeed_shm::{RingProducer, SegmentName, ShmError};
use std::sync::Arc;
use std::time::Duration;

/// A running crypto feed.
pub type Feed = crate::feed::Feed<FeedError>;

/// How long one REST call — a ticket or a start book — may take.
pub const REST_TIMEOUT: Duration = Duration::from_secs(10);

/// What a blocking feed sleeps per round while disconnected.
const DISCONNECTED_TICK: Duration = Duration::from_millis(50);

/// Refused messages logged in full before the log thins to one in a thousand.
const FAILURES_LOGGED: u64 = 5;

type Rx = Receiver<VenueRouter, RingProducer>;

/// Creates every feed's ring and router, and starts its thread.
///
/// Nothing is dialled here — the thread connects, and a venue that is down
/// at start is a venue the thread reconnects to. Nothing is started if
/// anything fails; what was already created is dropped.
pub fn start(conf: &CryptoConf, opts: Options, stop: Arc<AtomicBool>) -> Result<Vec<Feed>, StartError> {
    let tls = Tls::new();
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
                feed.name,
                feed.ring,
                ring.drops()
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

        let router = feed.router().map_err(|source| StartError::Router { feed: feed.name.clone(), source })?;
        // A venue that issues tickets has no address yet; the thread asks
        // for one before its first connect. The placeholder resolves to
        // nothing, so a ticket that never arrives is a connect failure in
        // the counters rather than a dial to somewhere unintended.
        let endpoint = feed.endpoint().unwrap_or_else(|| Endpoint::new("ticket.invalid", 443, "/", true));
        for sub in router.subscriptions_list() {
            info!(
                "{}: {} {} · price scale {} · qty scale {} · {}{}",
                feed.name,
                sub.instrument.venue().as_str(),
                String::from_utf8_lossy(sub.instrument.symbol_bytes()),
                sub.instrument.price_scale().decimals(),
                sub.instrument.qty_scale().decimals(),
                sub.channels,
                sub.depth.map_or(String::new(), |d| format!(" · depth {d}")),
            );
        }
        let rx = Receiver::new(feed.receiver_config(conf.health), endpoint, tls.clone(), router, ring);
        ready.push((feed, rx));
    }

    let mut feeds = Vec::with_capacity(ready.len());
    for (feed, rx) in ready {
        let name = feed.name.clone();
        let stop = Arc::clone(&stop);
        let conf = feed.clone();
        let tls = tls.clone();
        let handle = std::thread::Builder::new()
            .name(name.clone())
            .spawn(move || run(&conf, opts, rx, &tls, &stop))
            .map_err(|source| StartError::Spawn { feed: name.clone(), source })?;
        feeds.push(Feed::new(name, handle));
    }
    Ok(feeds)
}

/// The feed thread.
fn run(feed: &CryptoFeedConf, opts: Options, mut rx: Rx, tls: &Tls, stop: &AtomicBool) -> Result<(), FeedError> {
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
        "{name}: {} · burst {} · ping {} s · reconnect {} s · heartbeat {} ms · stale {} ms",
        match cfg.mode { Mode::Spin => "spin", Mode::Block => "block" },
        cfg.burst,
        cfg.ping_interval_ns / 1_000_000_000,
        cfg.reconnect_ns / 1_000_000_000,
        cfg.record_heartbeat_ns / 1_000_000,
        cfg.stale_ns / 1_000_000,
    );
    match feed.endpoint() {
        Some(ep) => info!("{name}: dialling {ep}"),
        None => info!("{name}: address comes from a ticket"),
    }

    let spin = cfg.mode == Mode::Spin;
    let needs_ticket = rx.pipeline().router().ticket().is_some();
    let mut ticket_due = needs_ticket;
    let mut ticket_retry_ns: u64 = 0;
    let mut opens_seen = 0u64;
    let mut disconnects_seen = 0u64;
    let mut rounds: u32 = 0;
    let mut next_report = clock::now_ns().saturating_add(opts.report_ns);

    while !stop.load(Ordering::Relaxed) {
        // ── the ticket, before a connect that needs one ───────────────
        if ticket_due && rx.state() == LinkState::Disconnected {
            let now = clock::now_ns();
            if now >= ticket_retry_ns {
                match fetch_ticket(&mut rx, tls) {
                    Ok(ep) => {
                        info!("{name}: ticket → {}://{}:{}{}", if ep.tls { "wss" } else { "ws" }, ep.host, ep.port, path_head(&ep.path));
                        rx.set_endpoint(ep);
                        ticket_due = false;
                    }
                    Err(e) => {
                        warn!("{name}: ticket: {e}");
                        ticket_retry_ns = now.saturating_add(cfg.reconnect_ns);
                    }
                }
            }
        }

        // ── one round ─────────────────────────────────────────────────
        if let Some(e) = rx.poll_once() {
            if e.is_closed() {
                info!("{name}: {e}");
            } else {
                warn!("{name}: {e}");
            }
        }

        let stats = *rx.stats();
        // A refused message: the first few in full, then one in a thousand,
        // so a venue that changed its protocol is visible in the log
        // without a venue sending junk at line rate filling the disk.
        if let Some(f) = rx.take_failure()
            && (stats.decode_failed <= FAILURES_LOGGED || stats.decode_failed.is_multiple_of(1000))
        {
            warn!(
                "{name}: refused #{}: {} · {} bytes: {}",
                stats.decode_failed,
                f.error,
                f.total,
                String::from_utf8_lossy(f.head())
            );
        }
        if stats.disconnects > disconnects_seen {
            disconnects_seen = stats.disconnects;
            ticket_due = needs_ticket;
        }
        if stats.opens > opens_seen {
            opens_seen = stats.opens;
            info!("{name}: connected ({} opens so far)", stats.opens);
            subscribe(name, &mut rx);
            start_books(name, &mut rx, tls);
        }

        // ── between rounds ────────────────────────────────────────────
        if !spin && rx.state() == LinkState::Disconnected {
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
    rx.shutdown(jeed_crypto::recv::CLOSE_GOING_AWAY);
    report(name, &rx);
    Ok(())
}

/// The ticket, from the router's request to the router's reading of it.
fn fetch_ticket(rx: &mut Rx, tls: &Tls) -> Result<Endpoint, RestError> {
    let request = rx.pipeline().router().ticket().expect("checked by the caller");
    let response = http::fetch(tls, &request, REST_TIMEOUT).map_err(RestError::Http)?;
    if !response.ok() {
        return Err(RestError::Status(response.status));
    }
    rx.router_mut().endpoint_from_ticket(&response.body).map_err(RestError::Ticket)
}

/// Sends what the router wants sent after an open.
fn subscribe(name: &str, rx: &mut Rx) {
    let messages = rx.pipeline().router().subscriptions();
    for msg in &messages {
        if let Err(e) = rx.send_text(msg) {
            warn!("{name}: subscribe: {e}");
            return;
        }
    }
    info!("{name}: sent {} subscription message(s)", messages.len());
}

/// Fetches and publishes the start books the router asks for.
fn start_books(name: &str, rx: &mut Rx, tls: &Tls) {
    for book in rx.pipeline().router().rest_books() {
        let symbol = rx
            .pipeline()
            .router()
            .subscriptions_list()
            .get(book.instrument)
            .map_or_else(String::new, |s| String::from_utf8_lossy(s.instrument.symbol_bytes()).into_owned());
        let response = match http::fetch(tls, &book.request, REST_TIMEOUT) {
            Ok(r) if r.ok() => r,
            Ok(r) => {
                warn!("{name}: start book {symbol}: {} answered {}", book.request, r.status);
                continue;
            }
            Err(e) => {
                warn!("{name}: start book {symbol}: {}: {e}", book.request);
                continue;
            }
        };
        match rx.ingest_rest(book.instrument, &response.body, clock::now_ns()) {
            Outcome::Published { records } => {
                info!("{name}: start book {symbol} · {} bytes · {records} record(s)", response.body.len());
            }
            Outcome::Failed(e) => warn!("{name}: start book {symbol}: {e}"),
        }
    }
}

/// One line of counters.
fn report(name: &str, rx: &Rx) {
    let s = rx.stats();
    let ring = rx.pipeline().sink();
    info!(
        "{name}: {} · msg {} · pub {} · hb {} · stale {} · fail {} · rest {} · frames {} (frag {} bin {}) \
         · ping {}/{} pong {}/{} keep {} · opens {} · disc {} · connfail {} · proto {} · ring seq {} drops {}",
        match rx.state() { LinkState::Open => "open", LinkState::Disconnected => "disconnected" },
        s.messages,
        s.published,
        s.heartbeats,
        s.stale,
        s.decode_failed,
        s.rest,
        s.frames,
        s.fragments,
        s.binary,
        s.pings_sent,
        s.pings,
        s.pongs_sent,
        s.pongs,
        s.keepalives_sent,
        s.opens,
        s.disconnects,
        s.connect_failures,
        s.protocol_errors,
        ring.next_seq(),
        ring.drops(),
    );
}

/// The path up to its query — a KuCoin token is not for the log.
fn path_head(path: &str) -> &str {
    path.split('?').next().unwrap_or(path)
}

/// A REST call on the router's behalf failed.
#[derive(Debug)]
enum RestError {
    Http(HttpError),
    Status(u16),
    Ticket(RouterError),
}

impl fmt::Display for RestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(e) => write!(f, "{e}"),
            Self::Status(code) => write!(f, "answered {code}"),
            Self::Ticket(e) => write!(f, "{e}"),
        }
    }
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

/// Why nothing was started.
#[derive(Debug)]
pub enum StartError {
    /// The segment could not be created.
    Ring {
        /// Feed.
        feed: String,
        /// Why.
        source: ShmError,
    },

    /// The venue refused the feed's instruments.
    Router {
        /// Feed.
        feed: String,
        /// Why.
        source: crate::conf::RuleError,
    },

    /// The thread could not be created.
    Spawn {
        /// Feed.
        feed: String,
        /// Why.
        source: std::io::Error,
    },
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ring { feed, source } => write!(f, "{feed}: ring: {source}"),
            Self::Router { feed, source } => write!(f, "{feed}: {source}"),
            Self::Spawn { feed, source } => write!(f, "{feed}: thread: {source}"),
        }
    }
}

impl std::error::Error for StartError {}
