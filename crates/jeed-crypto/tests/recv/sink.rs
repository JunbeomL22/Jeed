//! A [`RecordSink`] that behaves like the ring, and the tests that say so.
//!
//! The same double `jeed_fix::tests::recv::sink` uses, and for the same
//! reason. A `Vec` that only ever saw finished records would hide the one bug
//! the real slot is shaped to prevent: a router that writes half a record and
//! fails. So the buffer is **reused** exactly as a ring slot is, and a failed
//! `publish` leaves its wreckage behind for the next one to overwrite.

use jeed_wire::{RecordSink, WireRecord};

/// Collects published records, reusing one buffer the way a ring slot is
/// reused.
#[derive(Debug)]
pub struct Collect {
    /// Records that were committed, in order.
    pub records: Vec<WireRecord>,

    /// What the sink was told was dropped before it.
    pub drops: u64,

    /// The slot, kept across calls and never cleared.
    slot: WireRecord,
}

impl Collect {
    /// An empty sink.
    pub fn new() -> Self {
        Self { records: Vec::new(), drops: 0, slot: WireRecord::zeroed() }
    }

    /// The last record published.
    pub fn last(&self) -> &WireRecord {
        self.records.last().expect("nothing published")
    }

    /// How many records were published.
    pub fn len(&self) -> usize {
        self.records.len()
    }
}

impl RecordSink for Collect {
    fn publish<E>(&mut self, fill: impl FnOnce(&mut WireRecord) -> Result<(), E>) -> Result<(), E> {
        fill(&mut self.slot)?;
        self.records.push(self.slot);
        Ok(())
    }

    fn note_drops(&mut self, n: u64) {
        self.drops += n;
    }
}

#[test]
fn a_failed_fill_publishes_nothing() {
    let mut sink = Collect::new();
    let out: Result<(), &str> = sink.publish(|rec| {
        rec.header.depth = 7;
        Err("router gave up")
    });

    assert_eq!(out, Err("router gave up"));
    assert_eq!(sink.len(), 0, "a half-filled record must not be published");
}

#[test]
fn the_slot_keeps_the_previous_lap_like_the_ring_does() {
    let mut sink = Collect::new();
    let _: Result<(), ()> = sink.publish(|rec| {
        rec.header.depth = 5;
        Ok(())
    });
    let _: Result<(), ()> = sink.publish(|_| Ok(()));

    assert_eq!(sink.records[1].header.depth, 5, "the slot was not cleared between publishes");
}
