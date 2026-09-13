//! `jeed_shm::ring` — segment arithmetic.

use jeed_shm::{SEQ_IN_PROGRESS, segment_len};
use jeed_wire::{SEGMENT_HEADER_LEN, WIRE_ALIGN, WIRE_RECORD_LEN, WireRecord};

#[test]
fn a_segment_is_a_header_followed_by_whole_records() {
    assert_eq!(segment_len(0), SEGMENT_HEADER_LEN);
    assert_eq!(segment_len(1), SEGMENT_HEADER_LEN + WIRE_RECORD_LEN);
    assert_eq!(segment_len(1 << 16), 128 + 65_536 * 640);
}

#[test]
fn every_slot_lands_on_a_cache_line() {
    // The mapping base is 64 KiB aligned, so it is the header size and the
    // record size that have to cooperate.
    assert_eq!(SEGMENT_HEADER_LEN % WIRE_ALIGN, 0);
    assert_eq!(WIRE_RECORD_LEN % WIRE_ALIGN, 0);
    assert_eq!(align_of::<WireRecord>(), WIRE_ALIGN);
}

#[test]
fn the_in_progress_sentinel_cannot_collide_with_a_real_sequence() {
    // At one record per nanosecond, u64::MAX is 584 years of feed.
    assert_eq!(SEQ_IN_PROGRESS, u64::MAX);
    let ns_per_year = 365.25 * 24.0 * 3600.0 * 1e9;
    assert!(SEQ_IN_PROGRESS as f64 / ns_per_year > 500.0);
}

#[test]
fn the_sequence_field_is_naturally_aligned_inside_a_slot() {
    // What makes the per-slot seqlock a single atomic load, not a torn pair.
    let off = core::mem::offset_of!(WireRecord, header.producer_seq);
    assert_eq!(off, 8);
    assert_eq!(off % size_of::<u64>(), 0);
    assert_eq!((SEGMENT_HEADER_LEN + off) % size_of::<u64>(), 0);
}

#[test]
fn a_sixty_four_kilo_slot_segment_is_a_sane_default() {
    // Sizing note for `conf/`: this is what one hot channel costs in RAM.
    let bytes = segment_len(1 << 16);
    assert!((40..44).contains(&(bytes / (1024 * 1024))), "{} MiB", bytes / (1024 * 1024));
}
