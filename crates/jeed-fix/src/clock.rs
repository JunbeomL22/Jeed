//! The one place this crate reads a clock.
//!
//! Unlike `jeed_krx::clock` there is nothing to assemble: FIX timestamps are
//! absolute (`YYYYMMDD-HH:MM:SS.fff`), so nothing here has to guess a date the
//! way KRX's dateless 매매처리시각 forces it to (`CLAUDE.md`). What is left is
//! the reading itself.

use jeed_wire::UnixNano;
use std::time::{SystemTime, UNIX_EPOCH};

/// Wall-clock nanoseconds since the Unix epoch.
///
/// **The one sanctioned `std::time` site in this crate.** Everywhere else takes
/// the reading as an argument, because it has to happen next to the `read` that
/// justifies it — a timestamp taken three branches later measures this code,
/// not the feed.
///
/// It is `SystemTime`, and therefore **not monotonic**: it steps when the
/// system clock is disciplined. Ordering authority on this feed is `MsgSeqNum`;
/// `recv_ns` is for measurement and labelling, and every difference taken
/// against it is `saturating_sub` (`documents/feed_handler.md` §6).
///
/// A clock before the epoch reads as `0` rather than panicking. A feed handler
/// that dies because the clock is wrong is worse than one that labels a record
/// with a zero.
#[inline]
pub fn now_ns() -> UnixNano {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as UnixNano,
        Err(_) => 0,
    }
}
