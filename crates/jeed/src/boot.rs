//! The producer's boot identity.
//!
//! A consumer that stayed attached across a handler restart has exactly one
//! way to notice: the segment header's `boot_id` changed
//! (`documents/feed_handler.md` §7). So the only property that matters is that
//! two runs never share one. It is not a secret and not a sequence — a
//! consumer compares it for equality and nothing else.

use core::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static CALLS: AtomicU64 = AtomicU64::new(0);

/// A fresh identity for this run.
///
/// Wall-clock nanoseconds, the process id and a per-process counter, passed
/// through a 64-bit mixer so that consecutive runs do not share high bits
/// and never zero — zero is what an uninitialised header reads as.
pub fn boot_id() -> u64 {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = u64::from(std::process::id());
    let n = CALLS.fetch_add(1, Ordering::Relaxed);

    let id = mix(ns ^ pid.rotate_left(32) ^ n.rotate_left(48));
    if id == 0 { 1 } else { id }
}

/// splitmix64's finaliser.
#[inline]
const fn mix(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}
