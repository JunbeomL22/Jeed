//! What every handler's feed thread has in common: how it is started, how
//! it is watched, how it ends.
//!
//! ```text
//! main thread                        feed threads
//! ───────────────────────────        ─────────────────────
//! start(conf) → Vec<Feed>            run … until stop
//! wait(feeds, &stop) ───────────────→ join
//!   signal → stop
//!   one died → stop, exit 3
//! ```
//!
//! `krx::start` and `crypto::start` build different receivers, but what
//! they hand back and how the binary waits on it is the same — and the
//! rule that one feed dying stops them all is a rule worth having in one
//! place.

use crate::{error, info, signal};
use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

/// How to start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// This run's identity, written into every segment header.
    pub boot_id: u64,

    /// Pin each feed thread to its `cores`. Off is for a development box
    /// whose cores are not the conf's.
    pub pin: bool,

    /// Cadence of the report line. Zero silences it.
    pub report_ns: u64,
}

/// How a feed thread can end, other than on request.
pub trait Death: fmt::Display {
    /// The receive loop panicked.
    fn panicked() -> Self;
}

/// A running feed.
#[derive(Debug)]
pub struct Feed<E> {
    /// The conf's `name`.
    pub name: String,
    handle: JoinHandle<Result<(), E>>,
}

impl<E: Death> Feed<E> {
    /// Wraps a started thread.
    pub fn new(name: String, handle: JoinHandle<Result<(), E>>) -> Self {
        Self { name, handle }
    }

    /// `true` once the thread has returned.
    #[inline]
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }

    /// Waits for the thread and reports how it ended.
    ///
    /// A panic in the receive loop is reported as [`Death::panicked`] rather
    /// than propagated: the main thread has other feeds to stop first.
    pub fn join(self) -> Result<(), E> {
        match self.handle.join() {
            Ok(result) => result,
            Err(_) => Err(E::panicked()),
        }
    }
}

/// Runs the main thread until every feed has stopped.
///
/// Mirrors the signal handler into `stop`, joins feeds as they finish, and
/// — if one ends with an error — asks the rest to stop too. Returns `true`
/// if any feed died: one feed down is the whole handler down, because a
/// consumer sees one `boot_id` per ring and should not be left with half a
/// market.
pub fn wait<E: Death>(mut feeds: Vec<Feed<E>>, stop: &AtomicBool) -> bool {
    let mut failed = false;
    loop {
        if signal::requested() && !stop.load(Ordering::Relaxed) {
            info!("stop requested");
            stop.store(true, Ordering::Relaxed);
        }

        if let Some(i) = feeds.iter().position(Feed::is_finished) {
            let feed = feeds.swap_remove(i);
            let name = feed.name.clone();
            match feed.join() {
                Ok(()) => info!("{name}: stopped"),
                Err(e) => {
                    error!("{name}: {e}");
                    failed = true;
                    stop.store(true, Ordering::Relaxed);
                }
            }
        }

        if feeds.is_empty() {
            return failed;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
