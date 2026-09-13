//! Lines on stderr, with a UTC timestamp.
//!
//! ```text
//! 2026-09-13T01:02:03.456Z I hot: ring jeed.krx.hot 16384 slots boot 0x1f3a…
//! 2026-09-13T01:02:13.456Z I hot: recv 12345 pub 12000 …
//! 2026-09-13T01:02:13.457Z W hot: never seen on any socket: B603F
//! ```
//!
//! No logging crate: a feed handler writes a start-up banner, one report line
//! per feed every few seconds, and whatever went wrong. That is `eprintln!`
//! with a clock in front of it. The timestamp is UTC because the box's local
//! zone is a deployment fact the binary should not have to know, and because
//! KRX and the crypto venues are already in different ones.
//!
//! **Not for the hot path.** A report line is a `write(2)` from the receive
//! thread; the loop pays it once per report interval, on purpose, and never
//! per message.

use core::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// Severity, printed as one letter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Something happened.
    Info,
    /// Something is off but the handler keeps going.
    Warn,
    /// Something stopped the handler, or a feed.
    Error,
}

impl Level {
    const fn letter(self) -> char {
        match self {
            Self::Info => 'I',
            Self::Warn => 'W',
            Self::Error => 'E',
        }
    }
}

/// Writes one line.
pub fn line(level: Level, args: fmt::Arguments<'_>) {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    eprintln!("{} {} {args}", Timestamp(ns), level.letter());
}

/// `info!("{} …", x)`.
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => { $crate::log::line($crate::log::Level::Info, format_args!($($arg)*)) };
}

/// `warn!("{} …", x)`.
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => { $crate::log::line($crate::log::Level::Warn, format_args!($($arg)*)) };
}

/// `error!("{} …", x)`.
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => { $crate::log::line($crate::log::Level::Error, format_args!($($arg)*)) };
}

/// Unix nanoseconds, displayed as `YYYY-MM-DDTHH:MM:SS.mmmZ`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp(pub u64);

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let secs = self.0 / 1_000_000_000;
        let millis = (self.0 % 1_000_000_000) / 1_000_000;
        let days = (secs / 86_400) as i64;
        let (y, m, d) = civil_from_days(days);
        let tod = secs % 86_400;
        write!(
            f,
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
            tod / 3600,
            (tod % 3600) / 60,
            tod % 60
        )
    }
}

/// Days since 1970-01-01 → (year, month, day). Howard Hinnant's algorithm.
pub const fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
