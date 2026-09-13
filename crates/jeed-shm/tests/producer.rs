//! `jeed_shm::producer` — claiming, publishing, and abandoning slots.

mod common;

use common::unique;
use jeed_shm::{RingProducer, ShmError, segment_len};
use jeed_wire::{
    RecordHeader, RecordSink, SEGMENT_MAGIC, Scale, TradePayload, Venue, WIRE_FORMAT_VERSION,
    WIRE_RECORD_LEN, WireKind, WireRecord,
};

fn trade(price: i64) -> WireRecord {
    let mut h = RecordHeader::new(WireKind::Trade, Venue::Krx, *b"KR4101V90009", 1_785_455_100_000);
    h.set_scales(Scale::S2, Scale::S0);
    WireRecord::new_trade(h, TradePayload::new(price, 1))
}

#[test]
fn capacity_must_be_a_non_zero_power_of_two() {
    // Not a style rule: it is what turns the slot index into a mask instead of
    // a 64-bit division in the hot loop.
    let name = unique("cap");
    assert_eq!(
        RingProducer::create(&name, 0, 1).unwrap_err(),
        ShmError::Capacity { requested: 0 }
    );
    assert_eq!(
        RingProducer::create(&name, 100, 1).unwrap_err(),
        ShmError::Capacity { requested: 100 }
    );
    assert!(RingProducer::create(&name, 128, 1).is_ok());
}

#[test]
fn create_writes_a_header_a_reader_would_accept() {
    let name = unique("hdr");
    let p = RingProducer::create(&name, 64, 0xfeed_0001).unwrap();
    let h = p.header();

    assert_eq!(h.check(), Ok(()));
    assert_eq!(h.magic, SEGMENT_MAGIC);
    assert_eq!(h.format_version, WIRE_FORMAT_VERSION);
    assert_eq!(h.record_size as usize, WIRE_RECORD_LEN);
    assert_eq!(h.capacity, 64);
    assert_eq!(h.boot_id, 0xfeed_0001);
    assert_eq!(h.write_cursor(), 0);
    assert_eq!(h.segment_len(), segment_len(64));
    assert!(!p.reused_existing_section());
}

#[test]
fn committing_advances_the_sequence_and_the_cursor() {
    let name = unique("commit");
    let mut p = RingProducer::create(&name, 8, 1).unwrap();
    assert_eq!(p.next_seq(), 0);

    for expected in 0..5u64 {
        let seq = p.push(&trade(100 + expected as i64));
        assert_eq!(seq, expected);
        assert_eq!(p.next_seq(), expected + 1);
        assert_eq!(p.header().write_cursor(), expected + 1);
    }
}

#[test]
fn the_sequence_stamped_on_the_record_is_the_producers_not_the_callers() {
    let name = unique("stamp");
    let mut p = RingProducer::create(&name, 8, 1).unwrap();

    let mut rec = trade(100);
    rec.header.set_producer_seq(9_999); // a caller's leftover
    p.push(&rec);
    p.push(&rec);

    let slot = p.slot();
    assert_eq!(slot.seq(), 2, "the ring numbers its own records");
}

#[test]
fn an_abandoned_slot_publishes_nothing_and_is_handed_out_again() {
    // The reaction to a decode failure: drop the slot. A half-filled record
    // must never reach the ring (`CLAUDE.md`).
    let name = unique("abandon");
    let mut p = RingProducer::create(&name, 8, 1).unwrap();

    {
        let mut slot = p.slot();
        *slot = trade(100);
        // ..decode fails here, so no commit..
    }
    assert_eq!(p.next_seq(), 0);
    assert_eq!(p.header().write_cursor(), 0);

    let seq = p.push(&trade(200));
    assert_eq!(seq, 0, "the abandoned sequence is reused, not burned");
}

#[test]
fn a_slot_is_written_in_place_with_no_staging_copy() {
    let name = unique("inplace");
    let mut p = RingProducer::create(&name, 8, 1).unwrap();

    let mut slot = p.slot();
    slot.zeroed();
    slot.header = trade(12_345).header;
    slot.set_trade(TradePayload::new(12_345, 7));
    let seq = slot.commit();

    assert_eq!(seq, 0);
    // Read it back out of the segment through a fresh consumer view.
    let rx = jeed_shm::RingConsumer::attach(&name).unwrap();
    assert_eq!(rx.next_seq(), 1, "attach sits at the live edge");
    assert_eq!(rx.published(), 1);
}

#[test]
fn the_ring_wraps_without_the_sequence_wrapping() {
    let name = unique("wrap");
    let mut p = RingProducer::create(&name, 4, 1).unwrap();
    for i in 0..20 {
        p.push(&trade(i));
    }
    assert_eq!(p.next_seq(), 20);
    assert_eq!(p.header().write_cursor(), 20);
    assert_eq!(p.capacity(), 4);
}

#[test]
fn a_restart_reattaches_to_the_live_section_with_a_new_boot_id() {
    let name = unique("restart");
    // A consumer holding the section open is what makes this case real.
    let _keep = jeed_shm::SharedMapping::create(&name, segment_len(8)).unwrap();

    let mut first = RingProducer::create(&name, 8, 0xaaaa).unwrap();
    first.push(&trade(100));
    assert_eq!(first.header().write_cursor(), 1);
    drop(first);

    let second = RingProducer::create(&name, 8, 0xbbbb).unwrap();
    assert!(second.reused_existing_section());
    assert_eq!(second.header().boot_id, 0xbbbb);
    assert_eq!(second.header().write_cursor(), 0, "the new run starts from zero");
    assert_eq!(second.next_seq(), 0);
}

#[test]
fn drops_are_counted_before_the_ring_not_inside_it() {
    // Nothing is ever dropped by the ring itself — the producer overwrites and
    // the consumer notices. This counter is for records discarded earlier.
    let name = unique("drops");
    let mut p = RingProducer::create(&name, 8, 1).unwrap();
    assert_eq!(p.drops(), 0);

    p.note_drops(3);
    for i in 0..20 {
        p.push(&trade(i));
    }
    assert_eq!(p.drops(), 3, "lapping the ring is not a producer drop");
}

#[test]
fn a_producer_can_be_moved_onto_a_pinned_thread() {
    let name = unique("send");
    let mut p = RingProducer::create(&name, 8, 1).unwrap();
    let seq = std::thread::spawn(move || p.push(&trade(1))).join().unwrap();
    assert_eq!(seq, 0);
}

// ── RecordSink ──────────────────────────────────────────────────────────────

#[test]
fn publishing_through_the_sink_commits_the_slot() {
    // The handlers never name this crate: they are generic over `RecordSink`,
    // and the ring is what that resolves to in the binary.
    let name = unique("sink.ok");
    let mut p = RingProducer::create(&name, 8, 1).unwrap();

    let out: Result<(), ()> = p.publish(|rec| {
        *rec = trade(93_700);
        Ok(())
    });

    assert_eq!(out, Ok(()));
    assert_eq!(p.next_seq(), 1);
    assert_eq!(p.header().write_cursor(), 1);
}

#[test]
fn a_fill_that_fails_publishes_nothing_and_reuses_the_slot() {
    // The decode-failure path: the slot is claimed, half written, abandoned.
    // The cursor must not move and the sentinel must stay, or a consumer reads
    // a half-record wearing the previous lap's sequence.
    let name = unique("sink.err");
    let mut p = RingProducer::create(&name, 8, 1).unwrap();

    let out: Result<(), &str> = p.publish(|rec| {
        *rec = trade(93_700);
        Err("decoder gave up")
    });

    assert_eq!(out, Err("decoder gave up"));
    assert_eq!(p.next_seq(), 0, "the sequence was not consumed");
    assert_eq!(p.header().write_cursor(), 0, "nothing was published");

    // The same slot comes back, and publishing through it now works.
    let out: Result<(), ()> = p.publish(|rec| {
        *rec = trade(93_800);
        Ok(())
    });
    assert_eq!(out, Ok(()));
    assert_eq!(p.header().write_cursor(), 1);
}

#[test]
fn the_sink_reports_drops_to_the_segment() {
    let name = unique("sink.drops");
    let mut p = RingProducer::create(&name, 8, 1).unwrap();

    RecordSink::note_drops(&mut p, 2);
    RecordSink::note_drops(&mut p, 3);
    assert_eq!(p.drops(), 5);
}
