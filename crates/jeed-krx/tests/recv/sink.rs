//! A [`RecordSink`] that behaves like the ring, and the tests that say so.
//!
//! The double matters more than it looks. A `Vec` that only ever sees finished
//! records would hide the one bug the real slot is shaped to prevent: a decoder
//! that writes half a record and fails. So the buffer is **reused**, exactly as
//! a ring slot is — a failed `publish` leaves its wreckage behind for the next
//! one to overwrite, and any test that then reads a stale field is reading the
//! same bug a consumer would.

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
        Err("decoder gave up")
    });

    assert_eq!(out, Err("decoder gave up"));
    assert_eq!(sink.len(), 0, "a half-filled record must not be published");
}

#[test]
fn the_slot_keeps_the_previous_lap_like_the_ring_does() {
    // Not a nicety: this is what makes `a_failed_decode_does_not_leak_into_the_next_record`
    // in `pipeline` a real test rather than a tautology about a fresh `Vec`.
    let mut sink = Collect::new();
    let _: Result<(), ()> = sink.publish(|rec| {
        rec.header.depth = 5;
        Ok(())
    });
    let _: Result<(), ()> = sink.publish(|_| Ok(()));

    assert_eq!(sink.records[1].header.depth, 5, "the slot was not cleared between publishes");
}

#[test]
fn drops_accumulate() {
    let mut sink = Collect::new();
    sink.note_drops(1);
    sink.note_drops(3);
    assert_eq!(sink.drops, 4);
}
