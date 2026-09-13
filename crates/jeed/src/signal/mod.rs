//! The stop request: Ctrl-C on Windows, `SIGINT`/`SIGTERM` on POSIX.
//!
//! ```text
//!  Ctrl-C ──→ handler ──→ STOP = true ──→ every receive loop sees it on its next round
//! ```
//!
//! One flag, process-wide. A feed thread polls [`requested`] once per round —
//! a spinning loop within nanoseconds, a blocking one within its poll
//! timeout — and returns; the main thread joins them and exits. Nothing is
//! done in the handler but the store: a signal handler that logs, or frees,
//! or joins, is a handler that deadlocks one day.

#[cfg_attr(windows, path = "windows.rs")]
#[cfg_attr(unix, path = "posix.rs")]
mod imp;

use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};

static STOP: AtomicBool = AtomicBool::new(false);

/// Routes the platform's stop signals to the flag.
///
/// Call once, before the feed threads start. Calling it again is harmless.
pub fn install() -> Result<(), SignalError> {
    imp::install()
}

/// `true` once a stop has been requested, by a signal or by [`request`].
#[inline]
pub fn requested() -> bool {
    STOP.load(Ordering::Relaxed)
}

/// Requests a stop from inside the process — a feed that failed, or a test.
pub fn request() {
    STOP.store(true, Ordering::Relaxed);
}

/// Called from the OS handler.
pub(crate) fn from_handler() {
    STOP.store(true, Ordering::Relaxed);
}

/// The handler could not be installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignalError {
    /// The call, by its platform name.
    pub call: &'static str,
    /// `GetLastError()` on Windows, `errno` on POSIX.
    pub code: u32,
}

impl fmt::Display for SignalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} failed with code {}", self.call, self.code)
    }
}

impl std::error::Error for SignalError {}
