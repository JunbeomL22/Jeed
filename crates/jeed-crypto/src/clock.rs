//! The one place this crate reads a clock.
//!
//! Every decoder takes `recv_ns` as an argument and stamps venue time from the
//! message itself (`crate::time`), so nothing under `src/` but the receive loop
//! has a reason to ask what time it is. The reading lives here so that reason
//! is visible: one function, one caller, next to the `read` that justifies it.

use jeed_wire::UnixNano;
use std::time::{SystemTime, UNIX_EPOCH};

/// Wall-clock nanoseconds since the Unix epoch.
///
/// **The one sanctioned `std::time` site in this crate.** It is `SystemTime`,
/// and therefore **not monotonic**: it steps when the system clock is
/// disciplined. Ordering authority on the wire is `producer_seq`; `recv_ns` is
/// for measurement and labelling, and every difference taken against it is
/// `saturating_sub` (`CLAUDE.md`).
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
