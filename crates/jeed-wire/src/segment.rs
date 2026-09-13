//! Shared-memory segment header.
//!
//! Lives at offset 0 of a ring segment, followed by `capacity` records of
//! [`WIRE_RECORD_LEN`] bytes each. The first cache line is written once by the
//! producer at boot and is read-only afterwards; the second holds the
//! producer's live cursors, so reader polling never shares a line with the
//! constant fields.
//!
//! Attach protocol (`documents/feed_handler.md` §5): the reader calls
//! [`SegmentHeader::check`] and **refuses** the segment on any mismatch — a
//! layout disagreement must blow up, not be read wrong. A changed `boot_id`
//! means the producer restarted: the reader resets its expected
//! `producer_seq` and treats its book as stale.

use crate::error::WireError;
use crate::{WIRE_ALIGN, WIRE_FORMAT_VERSION, WIRE_RECORD_LEN};
use core::mem::size_of;
use core::sync::atomic::{AtomicU64, Ordering};

/// Segment magic: ASCII `FE_WIRE1` as a little-endian `u64`.
pub const SEGMENT_MAGIC: u64 = u64::from_le_bytes(*b"FE_WIRE1");

/// Size of the segment header in bytes (two cache lines).
pub const SEGMENT_HEADER_LEN: usize = 128;

/// Segment header (128 bytes).
#[derive(Debug)]
#[repr(C, align(64))]
pub struct SegmentHeader {
    /// [`SEGMENT_MAGIC`].
    pub magic: u64,

    /// [`WIRE_FORMAT_VERSION`] of the producer.
    pub format_version: u32,

    /// [`WIRE_RECORD_LEN`] of the producer.
    pub record_size: u32,

    /// Number of record slots following this header.
    pub capacity: u64,

    /// Producer boot identity; changes on every producer restart.
    pub boot_id: u64,

    /// Explicit padding to the end of the first cache line.
    pub _pad0: [u8; 32],

    /// Next `producer_seq` to be written. Records `< write_cursor` are
    /// published. Slot of sequence `s` is `s % capacity`.
    write_cursor: AtomicU64,

    /// Records the producer failed to write (ring full). Diagnostic only;
    /// readers detect their own gaps from `producer_seq`.
    drop_counter: AtomicU64,

    /// Explicit padding to the end of the second cache line.
    pub _pad1: [u8; 48],
}

const _: () = assert!(size_of::<SegmentHeader>() == SEGMENT_HEADER_LEN);
const _: () = assert!(align_of::<SegmentHeader>() == WIRE_ALIGN);
const _: () = assert!(core::mem::offset_of!(SegmentHeader, write_cursor) == 64);
// Each cache line is exactly filled: no implicit padding.
const _: () =
    assert!(size_of::<u64>() + 2 * size_of::<u32>() + 2 * size_of::<u64>() + 32 == WIRE_ALIGN);
const _: () = assert!(2 * size_of::<AtomicU64>() + 48 == WIRE_ALIGN);

impl SegmentHeader {
    /// Header for a fresh segment of `capacity` slots with cursors at zero.
    pub const fn new(capacity: u64, boot_id: u64) -> Self {
        Self {
            magic: SEGMENT_MAGIC,
            format_version: WIRE_FORMAT_VERSION,
            record_size: WIRE_RECORD_LEN as u32,
            capacity,
            boot_id,
            _pad0: [0; 32],
            write_cursor: AtomicU64::new(0),
            drop_counter: AtomicU64::new(0),
            _pad1: [0; 48],
        }
    }

    /// Attach-time check. Rejects a segment produced by a binary with a
    /// different wire layout, or with no slots.
    pub const fn check(&self) -> Result<(), WireError> {
        if self.magic != SEGMENT_MAGIC {
            return Err(WireError::Magic { found: self.magic });
        }
        if self.format_version != WIRE_FORMAT_VERSION {
            return Err(WireError::FormatVersion {
                expected: WIRE_FORMAT_VERSION,
                found: self.format_version,
            });
        }
        if self.record_size as usize != WIRE_RECORD_LEN {
            return Err(WireError::RecordSize {
                expected: WIRE_RECORD_LEN as u32,
                found: self.record_size,
            });
        }
        if self.capacity == 0 {
            return Err(WireError::Capacity);
        }
        Ok(())
    }

    /// Total segment size in bytes for this header's capacity.
    #[inline]
    pub const fn segment_len(&self) -> usize {
        SEGMENT_HEADER_LEN + self.capacity as usize * WIRE_RECORD_LEN
    }

    /// Byte offset of the slot holding `producer_seq`.
    #[inline]
    pub const fn slot_offset(&self, producer_seq: u64) -> usize {
        SEGMENT_HEADER_LEN + (producer_seq % self.capacity) as usize * WIRE_RECORD_LEN
    }

    /// Published cursor (acquire): records with `producer_seq < cursor` are
    /// complete.
    #[inline]
    pub fn write_cursor(&self) -> u64 {
        self.write_cursor.load(Ordering::Acquire)
    }

    /// Publishes records up to (excluding) `next` (release). The producer must
    /// have finished writing slot `next - 1` before calling.
    #[inline]
    pub fn publish(&self, next: u64) {
        self.write_cursor.store(next, Ordering::Release);
    }

    /// Records dropped by the producer so far.
    #[inline]
    pub fn drops(&self) -> u64 {
        self.drop_counter.load(Ordering::Relaxed)
    }

    /// Adds `n` to the drop counter.
    #[inline]
    pub fn note_drops(&self, n: u64) {
        self.drop_counter.fetch_add(n, Ordering::Relaxed);
    }
}
