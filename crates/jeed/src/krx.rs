//! The KRX handler: conf → rings → sockets → one thread per feed.
//!
//! ```text
//!  main thread                          feed thread "hot"        feed thread "cold"
//!  ─────────────────────────────        ──────────────────       ───────────────────
//!  KrxConf::load, validate
//!  for each feed:
//!    RingProducer::create   ─┐
//!    Receiver::new (joins)  ─┼─ move ─→  pin to core 2            pin to cores 4-7
//!  spawn                    ─┘          loop { poll_once }        loop { poll_once }
//!  wait for stop / a death              report every 10 s         report every 10 s
//!  join, exit
//! ```
//!
//! ## Everything that can fail, fails before any thread starts
//!
//! Rings are created and sockets joined on the main thread, feed by feed, so
//! a bad group address in the second feed is reported with nothing running —
//! not after the first feed has started publishing to a ring a consumer may
//! already be reading. The receivers are then *moved* onto their threads;
//! `Receiver` and `RingProducer` are both `Send` and neither is shared.
//!
//! ## The report is written by the receive thread
//!
//! Like the heartbeat (§9), and for the same reason: a report from another
//! thread would keep describing a loop that had stopped. It costs one
//! `write(2)` per interval on the pinned core. The clock is read once every
//! 256 rounds on a spinning feed — a quiet round already reads it inside
//! `poll_once`, so the report check is not the expensive part.

use crate::conf::{FeedConf, KrxConf};
use crate::cpu::{self, CpuError};
use crate::feed::Death;
use crate::{info, warn};
use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};
use jeed_krx::clock;
use jeed_krx::recv::{IsinFilter, Mode, NetError, Receiver, TrCodeFilter};
use jeed_shm::{RingProducer, SegmentName, ShmError};
use std::sync::Arc;

pub use crate::feed::Options;

/// A running KRX feed.
pub type Feed = crate::feed::Feed<FeedError>;

/// Creates every feed's ring, joins its sockets, and starts its thread.
///
/// Threads stop when `stop` becomes `true`. Nothing is started if anything
/// fails; what was already created is dropped.
pub fn start(
    conf: &KrxConf,
    isins: &IsinFilter,
    opts: Options,
    stop: Arc<AtomicBool>,
) -> Result<Vec<Feed>, StartError> {
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

        let trcodes = TrCodeFilter::new(feed.trcodes.iter().copied());
        let receiver = Receiver::new(
            feed.receiver_config(conf.health),
            &feed.sockets,
            TrCodeFilter::new(feed.trcodes.iter().copied()),
            IsinFilter::new(isins.isins().iter().copied()),
            ring,
        )
        .map_err(|source| StartError::Net { feed: feed.name.clone(), source })?;
        for socket in receiver.channels() {
            info!("{}: joined {}", feed.name, socket.endpoint);
        }
        info!("{}: keeping {} trcodes", feed.name, feed.trcodes.len());

        ready.push((feed, receiver, trcodes));
    }

    let mut feeds = Vec::with_capacity(ready.len());
    for (feed, receiver, trcodes) in ready {
        let name = feed.name.clone();
        let stop = Arc::clone(&stop);
        let conf = feed.clone();
        let handle = std::thread::Builder::new()
            .name(name.clone())
            .spawn(move || run(&conf, opts, receiver, &trcodes, &stop))
            .map_err(|source| StartError::Spawn { feed: name.clone(), source })?;
        feeds.push(Feed::new(name, handle));
    }
    Ok(feeds)
}

/// The feed thread.
fn run(
    feed: &FeedConf,
    opts: Options,
    mut rx: Receiver<RingProducer>,
    trcodes: &TrCodeFilter,
    stop: &AtomicBool,
) -> Result<(), FeedError> {
    let name = &feed.name;
    if opts.pin {
        let mask = feed.mask().ok_or(CpuError::BeyondMask)?;
        cpu::pin_current_thread(mask)?;
        info!("{name}: pinned to {:?} · now on processor {}", feed.cores, cpu::current_processor());
    } else {
        info!("{name}: not pinned (--no-pin)");
    }
    info!("{name}: {} · burst {} · heartbeat {} ms · stale {} ms",
        match feed.mode { Mode::Spin => "spin", Mode::Block => "block" },
        rx.config().burst,
        rx.config().heartbeat_ns / 1_000_000,
        rx.config().stale_ns / 1_000_000,
    );

    let spin = feed.mode == Mode::Spin;
    let mut rounds: u32 = 0;
    let mut next_report = clock::now_ns().saturating_add(opts.report_ns);

    while !stop.load(Ordering::Relaxed) {
        if let Err(e) = rx.poll_once() {
            report(name, &rx, trcodes);
            return Err(FeedError::Net(e));
        }
        rounds = rounds.wrapping_add(1);
        if opts.report_ns != 0 && (!spin || rounds & 0xFF == 0) {
            let now = clock::now_ns();
            if now >= next_report {
                report(name, &rx, trcodes);
                next_report = now.saturating_add(opts.report_ns);
            }
        }
    }

    info!("{name}: stopping");
    report(name, &rx, trcodes);
    for socket in rx.channels() {
        info!(
            "{name}: {} · recv {} · pub {} · errors {}",
            socket.endpoint, socket.received, socket.published, socket.errors
        );
    }
    Ok(())
}

/// One line of counters, and the wiring check.
fn report(name: &str, rx: &Receiver<RingProducer>, trcodes: &TrCodeFilter) {
    let s = rx.stats();
    let ring = rx.pipeline().sink();
    info!(
        "{name}: recv {} · pub {} · hb {} · stale {} · filtered {} · dropped {} \
         (short {} unknown {} len {} fail {}) · sockerr {} · ring seq {} drops {}",
        s.received,
        s.published,
        s.heartbeats,
        s.stale,
        s.filtered(),
        s.dropped(),
        s.too_short,
        s.unknown_trcode,
        s.wrong_length,
        s.decode_failed,
        s.socket_errors,
        ring.next_seq(),
        ring.drops(),
    );

    // A code that has arrived on no socket at all is either a market that has
    // not opened or a port that does not carry what the conf says it does —
    // the only check on that wiring there is (`documents/todo.md` §5).
    let mut unseen = String::new();
    for (i, code) in trcodes.codes().iter().enumerate() {
        if !rx.channels().iter().any(|c| c.saw(i)) {
            if !unseen.is_empty() {
                unseen.push(' ');
            }
            unseen.push_str(&code.to_string());
        }
    }
    if !unseen.is_empty() && s.received > 0 {
        warn!("{name}: never seen on any socket: {unseen}");
    }
}

/// Why a feed thread ended early.
#[derive(Debug)]
pub enum FeedError {
    /// Could not pin to the configured cores.
    Pin(CpuError),

    /// A socket failed in a way the loop does not absorb.
    Net(NetError),

    /// The receive loop panicked.
    Panicked,
}

impl fmt::Display for FeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pin(e) => write!(f, "could not pin: {e}"),
            Self::Net(e) => write!(f, "socket failure: {e}"),
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

    /// A socket could not be joined.
    Net {
        /// Feed.
        feed: String,
        /// Why.
        source: NetError,
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
            Self::Net { feed, source } => write!(f, "{feed}: {source}"),
            Self::Spawn { feed, source } => write!(f, "{feed}: thread: {source}"),
        }
    }
}

impl std::error::Error for StartError {}
