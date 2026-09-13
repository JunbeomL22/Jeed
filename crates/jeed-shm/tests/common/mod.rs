//! Shared helpers for the `jeed-shm` integration tests.

use jeed_shm::SegmentName;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A segment name unique to this process and this call.
///
/// Section names are kernel-global within the session, so a leftover segment
/// from a crashed test run would otherwise be picked up by the next one and
/// turn a failure into a confusing failure.
pub fn unique(tag: &str) -> SegmentName {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    SegmentName::local(&format!("jeed.test.{}.{tag}.{n}", std::process::id())).expect("valid name")
}
