//! The reading end of a segment.

use crate::error::ShmError;
use crate::mapping::SharedMapping;
use crate::name::SegmentName;
use crate::ring::{seq_cell, slot_offset};
use core::sync::atomic::{Ordering, fence};
use jeed_wire::{SEGMENT_HEADER_LEN, SegmentHeader, WireRecord};

/// Outcome of one [`RingConsumer::try_recv`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "the output record is only valid when this is `Record`"]
pub enum Recv {
    /// A record was copied into the output buffer.
    Record,

    /// Nothing has been published past the consumer's cursor.
    ///
    /// This is *not* a liveness signal: a quiet market looks identical to a
    /// dead producer. That is what heartbeat records are for (§9).
    Empty,

    /// The producer lapped the consumer. The cursor has been moved forward to
    /// the oldest record still in the ring; call again to read it.
    ///
    /// The count is records lost. Zero means the consumer was caught reading a
    /// slot the producer was overwriting and backed off without losing ground —
    /// a warning that it is at the edge, not yet a loss.
    Lagged(u64),

    /// The producer restarted: `boot_id` changed. The cursor has been re-seated
    /// at the new run's live edge.
    ///
    /// Everything the consumer built from this segment is stale, and the wire
    /// says nothing about how long the gap was (§5).
    Restarted {
        /// The new producer's boot identity.
        boot_id: u64,
    },
}

/// Single consumer for one segment.
///
/// Attaches read-only, so a bug on this side cannot corrupt the feed. Two
/// strategy threads read the same segment through **two separate**
/// `RingConsumer`s with independent cursors — never one reader forwarding to
/// the other, which would put a router back in the path
/// (`documents/feed_handler.md` §2).
#[derive(Debug)]
pub struct RingConsumer {
    map: SharedMapping,
    capacity: u64,
    mask: u64,
    boot_id: u64,
    next_seq: u64,
}

impl RingConsumer {
    /// Attaches to an existing segment, positioned at its **live edge**.
    ///
    /// Whatever is already in the ring is history, and a consumer that starts
    /// by replaying a lap of stale book state would act on prices that are
    /// minutes old. Refuses the segment outright on any layout disagreement
    /// (§5) rather than reading it wrong.
    pub fn attach(name: &SegmentName) -> Result<Self, ShmError> {
        let map = SharedMapping::open(name)?;
        if map.len() < SEGMENT_HEADER_LEN {
            return Err(ShmError::SegmentTooSmall {
                needed: SEGMENT_HEADER_LEN,
                mapped: map.len(),
            });
        }

        // SAFETY: at least `SEGMENT_HEADER_LEN` mapped bytes at offset 0, and
        // the mapping base is page aligned.
        let hdr = unsafe { &*map.as_ptr().cast::<SegmentHeader>() };
        hdr.check()?;

        let capacity = hdr.capacity;
        if !capacity.is_power_of_two() {
            return Err(ShmError::Capacity { requested: capacity });
        }
        // The header is written by another process: its claimed extent is a
        // claim, and slicing memory on it without checking would be the whole
        // ballgame.
        let needed = hdr.segment_len();
        if map.len() < needed {
            return Err(ShmError::SegmentTooSmall { needed, mapped: map.len() });
        }

        let boot_id = hdr.boot_id;
        let next_seq = hdr.write_cursor();
        Ok(Self { map, capacity, mask: capacity - 1, boot_id, next_seq })
    }

    /// Segment header.
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

    /// Sequence this consumer will read next.
    #[inline]
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Producer boot identity as of the last [`try_recv`](Self::try_recv).
    #[inline]
    pub fn boot_id(&self) -> u64 {
        self.boot_id
    }

    /// Sequence one past the newest published record.
    #[inline]
    pub fn published(&self) -> u64 {
        self.header().write_cursor()
    }

    /// Records published but not yet read.
    ///
    /// A value approaching [`capacity`](Self::capacity) means the consumer is
    /// about to start losing records.
    #[inline]
    pub fn lag(&self) -> u64 {
        self.published().saturating_sub(self.next_seq)
    }

    /// Producer-side drop counter (records discarded before the ring).
    #[inline]
    pub fn drops(&self) -> u64 {
        self.header().drops()
    }

    /// Reads the next record into `out`.
    ///
    /// Does not validate the record: the sequence lock rules out a torn read,
    /// and beyond that the producer is trusted to have written a record it
    /// built. Call [`WireRecord::validate`] if the segment's producer is not
    /// under the same deployment control.
    pub fn try_recv(&mut self, out: &mut WireRecord) -> Recv {
        let hdr = self.header();

        let boot_id = hdr.boot_id;
        let published = hdr.write_cursor();
        if boot_id != self.boot_id {
            self.boot_id = boot_id;
            self.next_seq = published;
            return Recv::Restarted { boot_id };
        }

        let want = self.next_seq;
        if want >= published {
            return Recv::Empty;
        }

        // Everything below `published - capacity` has already been overwritten.
        if published - want > self.capacity {
            let safe = published - self.capacity;
            self.next_seq = safe;
            return Recv::Lagged(safe - want);
        }

        let rec =
            // SAFETY: `want & mask` is below `capacity`, and `attach` checked
            // that the mapping covers every slot.
            unsafe { self.map.as_ptr().add(slot_offset(want, self.mask)) }.cast::<WireRecord>();
        // SAFETY: `rec` addresses a live slot.
        let seq = unsafe { seq_cell(rec) };

        if seq.load(Ordering::Acquire) != want {
            return self.reseat(want);
        }

        // SAFETY: the slot is 576 initialised bytes of exactly this layout.
        // Volatile because the producer may be writing it: the check below is
        // only meaningful if the compiler cannot move the copy across it.
        *out = unsafe { rec.read_volatile() };

        fence(Ordering::Acquire);
        if seq.load(Ordering::Relaxed) != want {
            return self.reseat(want);
        }

        self.next_seq = want + 1;
        Recv::Record
    }

    /// Moves the cursor to the oldest still-readable record after losing a race
    /// with the producer over the slot at `want`.
    #[cold]
    fn reseat(&mut self, want: u64) -> Recv {
        let safe = self.published().saturating_sub(self.capacity);
        self.next_seq = safe.max(want);
        Recv::Lagged(self.next_seq - want)
    }
}
