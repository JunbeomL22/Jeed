//! Shared helpers for the `jeed-shm` integration tests.

use jeed_shm::{SegmentName, SharedMapping};
use std::ops::Deref;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A segment name unique to this process and this call, cleaned up on drop.
///
/// Names are kernel-global within the session, so a leftover segment from a
/// crashed test run would otherwise be picked up by the next one and turn a
/// failure into a confusing failure.
///
/// The cleanup is not symmetric, because the platforms are not: a Windows
/// section disappears with its last handle and [`unlink`](SharedMapping::unlink)
/// is a no-op there, while on POSIX the name stays in `/dev/shm` until somebody
/// removes it — including after a test that made a 40 MB ring.
pub fn unique(tag: &str) -> Name {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = SegmentName::local(&format!("jeed.test.{}.{tag}.{n}", std::process::id()))
        .expect("valid name");
    Name(name)
}

/// A [`SegmentName`] that unlinks itself when the test ends.
///
/// Derefs to the name, so it is passed exactly as a `SegmentName` would be.
#[derive(Debug)]
pub struct Name(SegmentName);

impl Deref for Name {
    type Target = SegmentName;

    fn deref(&self) -> &SegmentName {
        &self.0
    }
}

impl Drop for Name {
    fn drop(&mut self) {
        // The segment may never have been created — a test that only checked a
        // name is entitled to leave nothing behind.
        let _ = SharedMapping::unlink(&self.0);
    }
}
