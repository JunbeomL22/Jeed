//! The writing end of a segment.

use crate::error::ShmError;
use crate::mapping::SharedMapping;
use crate::name::SegmentName;
use crate::ring::{SEQ_IN_PROGRESS, seq_cell, slot_offset};
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{Ordering, fence};
use jeed_wire::{
    RecordSink, SEGMENT_HEADER_LEN, SEGMENT_MAGIC, SegmentHeader, WIRE_FORMAT_VERSION,
    WIRE_RECORD_LEN, WireRecord,
};

/// Single producer for one segment.
///
/// One pinned core, one receive thread, one `RingProducer`
/// (`documents/feed_handler.md` §3). The type is `Send` but not `Sync`: it is
/// moved onto the receive thread at startup and never shared.
#[derive(Debug)]
pub struct RingProducer {
    map: SharedMapping,
    capacity: u64,
    mask: u64,
    next_seq: u64,
    reused: bool,
}

impl RingProducer {
    /// Creates (or reattaches to) the segment and writes a fresh header.
    ///
    /// `capacity` must be a non-zero power of two — that is what lets the slot
    /// index be a mask instead of a division.
    ///
    /// `boot_id` identifies this run of the producer. It must differ from the
    /// previous run's: it is the only signal a consumer that stayed attached
    /// across a restart gets, and on seeing it change the consumer resets its
    /// cursor and treats its book as stale (§5).
    ///
    /// Reattaching happens when the section is still alive — typically because
    /// a consumer held it open while the producer crashed and restarted. The
    /// old records are left in place rather than zeroed: nothing can read them,
    /// because the write cursor is back at zero and a consumer only reads below
    /// it. The drop counter is likewise left alone, so it is cumulative over the
    /// life of the *section*, not of this run.
    ///
    /// A consumer attaching during this call may catch a half-written header.
    /// It then fails [`SegmentHeader::check`] and must retry — which is the
    /// correct reaction to a producer that is booting.
    pub fn create(name: &SegmentName, capacity: u64, boot_id: u64) -> Result<Self, ShmError> {
        if capacity == 0 || !capacity.is_power_of_two() {
            return Err(ShmError::Capacity { requested: capacity });
        }
        let needed = SEGMENT_HEADER_LEN + capacity as usize * WIRE_RECORD_LEN;
        let map = SharedMapping::create(name, needed)?;
        let reused = map.existed();

        let hp = map.as_ptr().cast::<SegmentHeader>();
        // SAFETY: `hp` addresses at least `SEGMENT_HEADER_LEN` bytes of mapped,
        // writable memory (checked by `SharedMapping::create`). The constants
        // are written through raw places, never a `&mut`, because a consumer
        // may hold a shared reference to the same header at this instant.
        unsafe {
            (&raw mut (*hp).magic).write_volatile(SEGMENT_MAGIC);
            (&raw mut (*hp).format_version).write_volatile(WIRE_FORMAT_VERSION);
            (&raw mut (*hp).record_size).write_volatile(WIRE_RECORD_LEN as u32);
            (&raw mut (*hp).capacity).write_volatile(capacity);
            (&raw mut (*hp).boot_id).write_volatile(boot_id);
            (&raw mut (*hp)._pad0).write_volatile([0; 32]);
        }
        // The constants must be in place before the cursor says there is
        // anything to read.
        fence(Ordering::Release);
        // SAFETY: as above; the cursor line is only ever touched atomically.
        unsafe { &*hp }.publish(0);

        Ok(Self { map, capacity, mask: capacity - 1, next_seq: 0, reused })
    }

    /// Segment header (constants plus the producer's cursors).
    #[inline]
    pub fn header(&self) -> &SegmentHeader {
        // SAFETY: the mapping is at least header-sized and outlives `self`.
        unsafe { &*self.map.as_ptr().cast::<SegmentHeader>() }
    }

    /// Number of slots.
    #[inline]
    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Sequence the next [`slot`](Self::slot) will carry.
    #[inline]
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// `true` if the section already existed when this producer started.
    ///
    /// Expected after a crash-restart with a consumer still attached.
    /// Unexpected at a cold start — it means a second producer is writing the
    /// same segment, which the SPSC discipline does not survive.
    #[inline]
    pub fn reused_existing_section(&self) -> bool {
        self.reused
    }

    /// Records the producer declined to write, cumulative over the life of the
    /// section. Diagnostic only — a consumer detects its own losses from
    /// `producer_seq`.
    #[inline]
    pub fn drops(&self) -> u64 {
        self.header().drops()
    }

    /// Notes `n` records dropped before the ring (a decode that failed, a
    /// datagram discarded). Nothing in the ring itself drops.
    #[inline]
    pub fn note_drops(&self, n: u64) {
        self.header().note_drops(n);
    }

    /// Claims the next slot to decode into.
    ///
    /// The slot is marked in-progress before the reference escapes, so a
    /// consumer reading it concurrently sees a slot under construction rather
    /// than the previous lap's record. It still holds that previous record's
    /// bytes: overwrite what you mean to publish, or call
    /// [`Slot::zeroed`].
    ///
    /// Dropping the [`Slot`] without [`commit`](Slot::commit) abandons it. The
    /// sequence stays at the in-progress sentinel and the write cursor does not
    /// move, so nothing observes the partial write and the same slot is handed
    /// out again next time. That is the intended reaction to a decode failure —
    /// never publish a half-filled record (`CLAUDE.md`).
    #[inline]
    pub fn slot(&mut self) -> Slot<'_> {
        let rec =
            // SAFETY: `next_seq & mask` is below `capacity`, so the offset is
            // inside the mapped segment.
            unsafe { self.map.as_ptr().add(slot_offset(self.next_seq, self.mask)) }
                .cast::<WireRecord>();
        // SAFETY: `rec` addresses a live slot.
        unsafe { seq_cell(rec) }.store(SEQ_IN_PROGRESS, Ordering::Relaxed);
        // Body writes must not become visible before the sentinel.
        fence(Ordering::Release);
        Slot { producer: self, rec }
    }

    /// Copies a finished record into the next slot and publishes it.
    ///
    /// Returns the sequence it was published under. The record's own
    /// `producer_seq` is overwritten.
    #[inline]
    pub fn push(&mut self, rec: &WireRecord) -> u64 {
        let mut slot = self.slot();
        *slot = *rec;
        slot.commit()
    }
}

/// The ring is the live sink. A handler generic over
/// [`RecordSink`] therefore names neither this crate nor Windows
/// (`jeed_wire::sink`).
impl RecordSink for RingProducer {
    /// Decodes straight into the slot: `fill` writes into shared memory with no
    /// staging copy, and an `Err` drops the [`Slot`] without committing, so the
    /// sequence stays at the in-progress sentinel and the write cursor does not
    /// move.
    #[inline]
    fn publish<E>(
        &mut self,
        fill: impl FnOnce(&mut WireRecord) -> Result<(), E>,
    ) -> Result<(), E> {
        let mut slot = self.slot();
        fill(&mut slot)?;
        slot.commit();
        Ok(())
    }

    #[inline]
    fn note_drops(&mut self, n: u64) {
        RingProducer::note_drops(self, n);
    }
}

/// A claimed, not yet published, ring slot.
///
/// Derefs to the [`WireRecord`] in shared memory, so a decoder writes straight
/// into the segment with no staging copy.
#[derive(Debug)]
pub struct Slot<'a> {
    producer: &'a mut RingProducer,
    rec: *mut WireRecord,
}

impl Slot<'_> {
    /// Sequence this slot will be published under.
    #[inline]
    pub fn seq(&self) -> u64 {
        self.producer.next_seq
    }

    /// Zeroes the slot, discarding the previous lap's record.
    ///
    /// Only worth it when the record kind about to be written leaves fields
    /// untouched; the `WireRecord` constructors already zero what they own.
    #[inline]
    pub fn zeroed(&mut self) -> &mut WireRecord {
        **self = WireRecord::zeroed();
        self
    }

    /// Stamps the sequence and publishes the slot.
    ///
    /// Returns the sequence published.
    #[inline]
    pub fn commit(self) -> u64 {
        let seq = self.producer.next_seq;
        // Everything written through the deref must be visible before the
        // sequence that advertises it.
        fence(Ordering::Release);
        // SAFETY: `rec` addresses the live slot claimed by `slot()`.
        unsafe { seq_cell(self.rec) }.store(seq, Ordering::Relaxed);

        self.producer.next_seq = seq + 1;
        self.producer.header().publish(seq + 1);
        seq
    }
}

impl Deref for Slot<'_> {
    type Target = WireRecord;

    #[inline]
    fn deref(&self) -> &WireRecord {
        // SAFETY: the slot is live for the life of the guard, and the guard
        // holds the producer's only mutable borrow, so nothing else in this
        // process touches it.
        unsafe { &*self.rec }
    }
}

impl DerefMut for Slot<'_> {
    #[inline]
    fn deref_mut(&mut self) -> &mut WireRecord {
        // SAFETY: as above.
        unsafe { &mut *self.rec }
    }
}
