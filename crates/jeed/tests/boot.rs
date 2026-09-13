//! `jeed::boot` — the run identity.

use jeed::boot_id;
use std::collections::HashSet;

#[test]
fn never_zero() {
    // Zero is what an uninitialised header reads as.
    for _ in 0..1000 {
        assert_ne!(boot_id(), 0);
    }
}

#[test]
fn consecutive_runs_do_not_collide() {
    // Two starts inside one nanosecond would share a clock reading; the
    // counter keeps them apart anyway.
    let ids: HashSet<u64> = (0..10_000).map(|_| boot_id()).collect();
    assert_eq!(ids.len(), 10_000);
}
