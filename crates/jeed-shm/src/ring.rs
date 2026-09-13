//! Slot arithmetic and the per-slot sequence lock shared by both ends.
//!
//! ## Why an overwriting ring and not a back-pressured one
//!
//! [`SegmentHeader`](jeed_wire::SegmentHeader) carries a write cursor and no
//! read cursor, and that is the design, not an omission. A back-pressured ring
//! drops the **newest** record when it fills — it keeps stale book state and
//! throws away the print that just moved the market. For a feed that is the
//! wrong trade. Here the producer never stalls and never chooses to drop; a
//! consumer that falls a full lap behind loses the **oldest** records and
//! finds out from the `producer_seq` in the record it does get
//! (`documents/feed_handler.md` §7).
//!
//! ## The one hazard that buys
//!
//! Overwriting means the producer can be rewriting a slot while the consumer
//! reads it — a torn record. The record's own `producer_seq` field is
//! therefore used as a sequence lock:
//!
//! ```text
//! producer                                consumer
//!   store seq = IN_PROGRESS                 load  seq   → must equal `want`
//!   fence(Release)                          copy the record
//!   ..write the record body..               fence(Acquire)
//!   fence(Release)                          load  seq   → must still equal `want`
//!   store seq = s
//!   publish(s + 1)
//! ```
//!
//! The sentinel matters: without it the slot would still hold the *previous
//! lap's* valid-looking sequence while its body was half-overwritten.

use core::sync::atomic::AtomicU64;
use jeed_wire::{SEGMENT_HEADER_LEN, WIRE_RECORD_LEN, WireRecord};

/// Sequence value marking a slot the producer is in the middle of writing.
///
/// `u64::MAX` is unreachable as a real sequence: at one record per nanosecond
/// it is 584 years of feed.
pub const SEQ_IN_PROGRESS: u64 = u64::MAX;

/// Byte offset of `producer_seq` within a record.
pub(crate) const SEQ_OFFSET: usize = core::mem::offset_of!(WireRecord, header.producer_seq);

// The sequence lock is only atomic if the field is naturally aligned inside a
// cache-line-aligned slot.
const _: () = assert!(SEQ_OFFSET.is_multiple_of(size_of::<u64>()));
const _: () = assert!(SEGMENT_HEADER_LEN.is_multiple_of(align_of::<WireRecord>()));
const _: () = assert!(WIRE_RECORD_LEN.is_multiple_of(align_of::<WireRecord>()));

/// Byte offset of the slot holding `seq` in a ring of `mask + 1` slots.
///
/// `mask` rather than `capacity % `: the capacity is required to be a power of
/// two precisely so this is an `and` and not a 64-bit division.
#[inline]
pub(crate) const fn slot_offset(seq: u64, mask: u64) -> usize {
    SEGMENT_HEADER_LEN + (seq & mask) as usize * WIRE_RECORD_LEN
}

/// The `producer_seq` field of a slot, viewed as an atomic.
///
/// # Safety
///
/// `rec` must point at a live record slot inside a mapped segment.
#[inline]
pub(crate) unsafe fn seq_cell<'a>(rec: *mut WireRecord) -> &'a AtomicU64 {
    // SAFETY: the caller guarantees the slot is live; `SEQ_OFFSET` is a
    // multiple of 8 and the slot is 64-byte aligned, so the field is naturally
    // aligned. Only this crate ever touches it, and only atomically.
    unsafe { AtomicU64::from_ptr(rec.cast::<u8>().add(SEQ_OFFSET).cast::<u64>()) }
}
