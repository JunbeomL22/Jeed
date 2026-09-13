//! `jeed_shm::consumer` — attaching, reading, and noticing what was missed.

mod common;

use common::unique;
use jeed_shm::{Recv, RingConsumer, RingProducer, SharedMapping, ShmError, segment_len};
use jeed_wire::{
    RecordHeader, Scale, TradePayload, Venue, WIRE_FORMAT_VERSION, WireError, WireKind, WireRecord,
};

fn trade(price: i64) -> WireRecord {
    let mut h = RecordHeader::new(WireKind::Trade, Venue::Krx, *b"KR4101V90009", 1_785_455_100_000);
    h.set_scales(Scale::S2, Scale::S0);
    WireRecord::new_trade(h, TradePayload::new(price, 1))
}

#[test]
fn attaching_to_a_segment_nobody_published_is_an_error() {
    let name = unique("absent");
    assert!(matches!(
        RingConsumer::attach(&name).unwrap_err(),
        ShmError::Os { call: "OpenFileMappingW", .. }
    ));
}

#[test]
fn a_segment_with_a_foreign_layout_is_refused_not_read() {
    let name = unique("foreign");
    let raw = SharedMapping::create(&name, segment_len(8)).unwrap();
    // A producer built against a different wire version.
    let p = raw.as_ptr().cast::<jeed_wire::SegmentHeader>();
    // SAFETY: the mapping covers the header.
    unsafe {
        (&raw mut (*p).magic).write(jeed_wire::SEGMENT_MAGIC);
        (&raw mut (*p).format_version).write(WIRE_FORMAT_VERSION + 1);
        (&raw mut (*p).record_size).write(jeed_wire::WIRE_RECORD_LEN as u32);
        (&raw mut (*p).capacity).write(8);
    }

    assert_eq!(
        RingConsumer::attach(&name).unwrap_err(),
        ShmError::Wire(WireError::FormatVersion {
            expected: WIRE_FORMAT_VERSION,
            found: WIRE_FORMAT_VERSION + 1,
        })
    );
}

#[test]
fn a_header_claiming_more_memory_than_exists_is_refused() {
    // The header comes from another process. Believing its capacity and then
    // indexing into the mapping on it is the whole ballgame.
    let name = unique("liar");
    let raw = SharedMapping::create(&name, segment_len(4)).unwrap();
    let p = raw.as_ptr().cast::<jeed_wire::SegmentHeader>();
    // SAFETY: the mapping covers the header.
    unsafe {
        (&raw mut (*p).magic).write(jeed_wire::SEGMENT_MAGIC);
        (&raw mut (*p).format_version).write(WIRE_FORMAT_VERSION);
        (&raw mut (*p).record_size).write(jeed_wire::WIRE_RECORD_LEN as u32);
        (&raw mut (*p).capacity).write(1 << 20);
    }

    assert!(matches!(
        RingConsumer::attach(&name).unwrap_err(),
        ShmError::SegmentTooSmall { .. }
    ));
}

#[test]
fn attaching_starts_at_the_live_edge_not_at_the_history() {
    let name = unique("edge");
    let mut tx = RingProducer::create(&name, 64, 1).unwrap();
    for i in 0..10 {
        tx.push(&trade(i));
    }

    let mut rx = RingConsumer::attach(&name).unwrap();
    assert_eq!(rx.next_seq(), 10);

    let mut out = WireRecord::zeroed();
    assert_eq!(rx.try_recv(&mut out), Recv::Empty, "history is not replayed");

    tx.push(&trade(999));
    assert_eq!(rx.try_recv(&mut out), Recv::Record);
    assert_eq!(out.trade().unwrap().price, 999);
    assert_eq!(out.header.producer_seq, 10);
}

#[test]
fn records_arrive_in_order_with_a_gapless_sequence() {
    let name = unique("order");
    let mut tx = RingProducer::create(&name, 64, 1).unwrap();
    let mut rx = RingConsumer::attach(&name).unwrap();
    let mut out = WireRecord::zeroed();

    for i in 0..40i64 {
        tx.push(&trade(i));
    }
    for i in 0..40i64 {
        assert_eq!(rx.try_recv(&mut out), Recv::Record);
        assert_eq!(out.header.producer_seq, i as u64);
        assert_eq!(out.trade().unwrap().price, i);
    }
    assert_eq!(rx.try_recv(&mut out), Recv::Empty);
    assert_eq!(rx.lag(), 0);
}

#[test]
fn interleaved_reads_and_writes_never_report_lag() {
    let name = unique("lockstep");
    let mut tx = RingProducer::create(&name, 4, 1).unwrap();
    let mut rx = RingConsumer::attach(&name).unwrap();
    let mut out = WireRecord::zeroed();

    // Capacity 4, but the consumer keeps up, so the ring wraps 25 times with
    // nothing lost.
    for i in 0..100i64 {
        tx.push(&trade(i));
        assert_eq!(rx.try_recv(&mut out), Recv::Record);
        assert_eq!(out.trade().unwrap().price, i);
    }
}

#[test]
fn a_lapped_consumer_is_told_how_much_it_lost() {
    let name = unique("lag");
    let mut tx = RingProducer::create(&name, 4, 1).unwrap();
    let mut rx = RingConsumer::attach(&name).unwrap();
    let mut out = WireRecord::zeroed();

    for i in 0..10i64 {
        tx.push(&trade(i));
    }
    // 10 published, 4 slots: sequences 0..6 are gone.
    assert_eq!(rx.lag(), 10);
    assert_eq!(rx.try_recv(&mut out), Recv::Lagged(6));
    assert_eq!(rx.next_seq(), 6);

    // ..and the next call delivers the oldest survivor, not a hole.
    assert_eq!(rx.try_recv(&mut out), Recv::Record);
    assert_eq!(out.header.producer_seq, 6);
    assert_eq!(out.trade().unwrap().price, 6);
}

#[test]
fn a_consumer_exactly_a_lap_behind_has_lost_nothing_yet() {
    let name = unique("edgecase");
    let mut tx = RingProducer::create(&name, 4, 1).unwrap();
    let mut rx = RingConsumer::attach(&name).unwrap();
    let mut out = WireRecord::zeroed();

    for i in 0..4i64 {
        tx.push(&trade(i));
    }
    assert_eq!(rx.lag(), 4);
    assert_eq!(rx.try_recv(&mut out), Recv::Record, "the oldest slot is still intact");
    assert_eq!(out.header.producer_seq, 0);
}

#[test]
fn a_producer_restart_is_reported_before_any_record() {
    let name = unique("reboot");
    let mut rx = {
        let mut tx = RingProducer::create(&name, 8, 0xaaaa).unwrap();
        tx.push(&trade(1));
        let rx = RingConsumer::attach(&name).unwrap();
        assert_eq!(rx.boot_id(), 0xaaaa);
        rx
        // `tx` dies here; `rx` keeps the section alive.
    };

    let mut tx = RingProducer::create(&name, 8, 0xbbbb).unwrap();
    tx.push(&trade(2));

    let mut out = WireRecord::zeroed();
    assert_eq!(rx.try_recv(&mut out), Recv::Restarted { boot_id: 0xbbbb });
    assert_eq!(rx.boot_id(), 0xbbbb);
    // Re-seated at the new run's live edge: what happened during the gap is
    // not recoverable from the wire, and the book is stale.
    assert_eq!(rx.next_seq(), 1);
    assert_eq!(rx.try_recv(&mut out), Recv::Empty);

    tx.push(&trade(3));
    assert_eq!(rx.try_recv(&mut out), Recv::Record);
    assert_eq!(out.trade().unwrap().price, 3);
    assert_eq!(out.header.producer_seq, 1);
}

#[test]
fn two_consumers_read_the_same_segment_with_independent_cursors() {
    // Two strategy threads, two cursors — not one reader forwarding to the
    // other (`documents/feed_handler.md` §2).
    let name = unique("fanout");
    let mut tx = RingProducer::create(&name, 64, 1).unwrap();
    let mut a = RingConsumer::attach(&name).unwrap();
    let mut b = RingConsumer::attach(&name).unwrap();
    let mut out = WireRecord::zeroed();

    for i in 0..5i64 {
        tx.push(&trade(i));
    }

    for i in 0..5i64 {
        assert_eq!(a.try_recv(&mut out), Recv::Record);
        assert_eq!(out.trade().unwrap().price, i);
    }
    assert_eq!(a.lag(), 0);
    assert_eq!(b.lag(), 5, "b has not moved because a read");

    assert_eq!(b.try_recv(&mut out), Recv::Record);
    assert_eq!(out.trade().unwrap().price, 0);
}

#[test]
fn a_slot_under_construction_is_not_delivered() {
    let name = unique("inflight");
    let mut tx = RingProducer::create(&name, 8, 1).unwrap();
    let mut rx = RingConsumer::attach(&name).unwrap();
    let mut out = WireRecord::zeroed();

    let mut slot = tx.slot();
    *slot = trade(42);
    assert_eq!(rx.try_recv(&mut out), Recv::Empty, "the cursor has not moved");

    slot.commit();
    assert_eq!(rx.try_recv(&mut out), Recv::Record);
    assert_eq!(out.trade().unwrap().price, 42);
}

#[test]
fn every_published_record_survives_a_concurrent_producer() {
    // The seqlock's reason for existing: the producer overwrites slots while
    // the consumer reads them.
    let name = unique("concurrent");
    const N: u64 = 200_000;
    let mut tx = RingProducer::create(&name, 1 << 12, 1).unwrap();
    let mut rx = RingConsumer::attach(&name).unwrap();

    let writer = std::thread::spawn(move || {
        for i in 0..N {
            tx.push(&trade(i as i64));
        }
        tx
    });

    let mut out = WireRecord::zeroed();
    let mut got = 0u64;
    let mut lost = 0u64;
    while got + lost < N {
        match rx.try_recv(&mut out) {
            Recv::Record => {
                // Whatever arrives must be internally consistent: the price is
                // the sequence. A torn read would break this.
                assert_eq!(out.trade().unwrap().price as u64, out.header.producer_seq);
                got += 1;
            }
            Recv::Lagged(n) => lost += n,
            Recv::Empty => std::hint::spin_loop(),
            Recv::Restarted { .. } => unreachable!("the producer does not restart"),
        }
    }
    writer.join().unwrap();
    assert_eq!(got + lost, N);
}
