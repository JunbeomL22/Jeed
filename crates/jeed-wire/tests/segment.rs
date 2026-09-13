//! `jeed_wire::segment` — the shared-memory segment header and its attach check.

use jeed_wire::{
    SEGMENT_HEADER_LEN, SEGMENT_MAGIC, SegmentHeader, WIRE_ALIGN, WIRE_FORMAT_VERSION,
    WIRE_RECORD_LEN, WireError,
};

#[test]
fn layout_is_two_cache_lines_with_the_cursors_on_the_second() {
    assert_eq!(size_of::<SegmentHeader>(), SEGMENT_HEADER_LEN);
    assert_eq!(size_of::<SegmentHeader>(), 128);
    assert_eq!(align_of::<SegmentHeader>(), WIRE_ALIGN);

    // The constant fields are read-only after boot; the producer's live
    // cursors must not share their cache line with a polling reader.
    assert_eq!(core::mem::offset_of!(SegmentHeader, magic), 0);
    assert_eq!(core::mem::offset_of!(SegmentHeader, _pad1), 80);
}

#[test]
fn magic_is_the_ascii_tag() {
    assert_eq!(SEGMENT_MAGIC.to_le_bytes(), *b"FE_WIRE1");
}

#[test]
fn a_fresh_segment_passes_its_own_check() {
    let h = SegmentHeader::new(1_024, 7);
    assert_eq!(h.check(), Ok(()));
    assert_eq!(h.format_version, WIRE_FORMAT_VERSION);
    assert_eq!(h.record_size as usize, WIRE_RECORD_LEN);
    assert_eq!(h.boot_id, 7);
    assert_eq!(h.write_cursor(), 0);
    assert_eq!(h.drops(), 0);
}

#[test]
fn a_producer_with_a_different_layout_is_refused() {
    // This is the whole point of the header: a layout disagreement must blow
    // up at attach, not be read as wrong prices.
    let mut h = SegmentHeader::new(16, 0);
    h.magic = 1;
    assert_eq!(h.check(), Err(WireError::Magic { found: 1 }));

    let mut h = SegmentHeader::new(16, 0);
    h.format_version = WIRE_FORMAT_VERSION + 1;
    assert_eq!(
        h.check(),
        Err(WireError::FormatVersion {
            expected: WIRE_FORMAT_VERSION,
            found: WIRE_FORMAT_VERSION + 1
        })
    );

    let mut h = SegmentHeader::new(16, 0);
    h.record_size = 512;
    assert_eq!(
        h.check(),
        Err(WireError::RecordSize { expected: WIRE_RECORD_LEN as u32, found: 512 })
    );
}

#[test]
fn an_empty_segment_is_refused() {
    assert_eq!(SegmentHeader::new(0, 0).check(), Err(WireError::Capacity));
}

#[test]
fn segment_length_is_the_header_plus_its_slots() {
    let h = SegmentHeader::new(4, 0);
    assert_eq!(h.segment_len(), SEGMENT_HEADER_LEN + 4 * WIRE_RECORD_LEN);
}

#[test]
fn slots_wrap_around_the_ring() {
    let h = SegmentHeader::new(4, 0);
    assert_eq!(h.slot_offset(0), SEGMENT_HEADER_LEN);
    assert_eq!(h.slot_offset(3), SEGMENT_HEADER_LEN + 3 * WIRE_RECORD_LEN);
    assert_eq!(h.slot_offset(4), SEGMENT_HEADER_LEN, "sequence 4 reuses slot 0");
    assert_eq!(h.slot_offset(9), h.slot_offset(1));
}

#[test]
fn every_slot_offset_is_cache_line_aligned() {
    let h = SegmentHeader::new(8, 0);
    for seq in 0..16 {
        assert_eq!(h.slot_offset(seq) % WIRE_ALIGN, 0, "slot for seq {seq}");
    }
}

#[test]
fn publishing_advances_the_cursor() {
    let h = SegmentHeader::new(16, 0);
    h.publish(1);
    assert_eq!(h.write_cursor(), 1);
    h.publish(5);
    assert_eq!(h.write_cursor(), 5);
}

#[test]
fn drops_accumulate() {
    // A full ring drops silently unless someone counts; this is that counter.
    let h = SegmentHeader::new(16, 0);
    h.note_drops(3);
    h.note_drops(2);
    assert_eq!(h.drops(), 5);
}
