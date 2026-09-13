//! `jeed_wire::header` — the 48-byte record header.

use jeed_wire::{
    RecordHeader, Scale, Venue, WIRE_HEADER_LEN, WIRE_MAX_DEPTH, WireError, WireKind, header_flags,
};

/// A header that passes validation, for tests that then break one field.
fn valid() -> RecordHeader {
    let mut h = RecordHeader::new(WireKind::Quote, Venue::Krx, *b"KR4A01690002", 1_000);
    h.set_scales(Scale::S2, Scale::S0);
    h
}

#[test]
fn layout_is_the_documented_one() {
    assert_eq!(size_of::<RecordHeader>(), WIRE_HEADER_LEN);
    assert_eq!(size_of::<RecordHeader>(), 48);
    assert_eq!(align_of::<RecordHeader>(), 8);

    assert_eq!(core::mem::offset_of!(RecordHeader, kind), 0);
    assert_eq!(core::mem::offset_of!(RecordHeader, venue), 1);
    assert_eq!(core::mem::offset_of!(RecordHeader, flags), 2);
    assert_eq!(core::mem::offset_of!(RecordHeader, depth), 3);
    assert_eq!(core::mem::offset_of!(RecordHeader, price_scale), 4);
    assert_eq!(core::mem::offset_of!(RecordHeader, qty_scale), 5);
    assert_eq!(core::mem::offset_of!(RecordHeader, producer_seq), 8);
    assert_eq!(core::mem::offset_of!(RecordHeader, recv_ns), 16);
    assert_eq!(core::mem::offset_of!(RecordHeader, venue_ns), 24);
    assert_eq!(core::mem::offset_of!(RecordHeader, isin), 32);
}

#[test]
fn zeroed_header_is_invalid() {
    // Kind 0 is unused precisely so an all-zero ring slot cannot pass as data.
    assert_eq!(RecordHeader::ZEROED.validate(), Err(WireError::Kind { found: 0 }));
}

#[test]
fn new_fills_identity_and_reception_time() {
    let h = valid();
    assert_eq!(h.kind(), Ok(WireKind::Quote));
    assert_eq!(h.venue(), Ok(Venue::Krx));
    assert_eq!(h.isin, *b"KR4A01690002");
    assert_eq!(h.recv_ns, 1_000);
    assert_eq!(h.validate(), Ok(()));
}

#[test]
fn scales_round_trip() {
    let h = valid();
    assert_eq!(h.price_scale(), Ok(Scale::S2));
    assert_eq!(h.qty_scale(), Ok(Scale::S0));
}

#[test]
fn unknown_enumerated_bytes_are_rejected() {
    let mut h = valid();
    h.kind = 200;
    assert_eq!(h.validate(), Err(WireError::Kind { found: 200 }));

    let mut h = valid();
    h.venue = 9;
    assert_eq!(h.validate(), Err(WireError::Venue { found: 9 }));

    let mut h = valid();
    h.price_scale = 9;
    assert_eq!(h.validate(), Err(WireError::Scale { found: 9 }));

    let mut h = valid();
    h.qty_scale = 100;
    assert_eq!(h.validate(), Err(WireError::Scale { found: 100 }));
}

#[test]
fn depth_beyond_the_layout_is_rejected() {
    let mut h = valid();
    h.set_depth(WIRE_MAX_DEPTH as u8);
    assert_eq!(h.validate(), Ok(()));

    h.set_depth(WIRE_MAX_DEPTH as u8 + 1);
    assert_eq!(
        h.validate(),
        Err(WireError::Depth { found: 11, max: WIRE_MAX_DEPTH as u8 })
    );
}

#[test]
fn venue_time_is_absent_until_marked_valid() {
    let mut h = valid();
    assert_eq!(h.venue_time(), None);
    assert_eq!(h.venue_age_ns(), None);

    h.set_venue_time(900);
    assert!(h.has(header_flags::VENUE_TIME_VALID));
    assert_eq!(h.venue_time(), Some(900));
}

#[test]
fn venue_age_saturates_instead_of_wrapping() {
    // Neither clock is monotonic and UnixNano is unsigned, so a venue stamp
    // ahead of our reception stamp must read as age 0 — not as ~584 years.
    let mut h = valid();
    h.recv_ns = 1_000;
    h.set_venue_time(5_000);
    assert_eq!(h.venue_age_ns(), Some(0));

    h.set_venue_time(400);
    assert_eq!(h.venue_age_ns(), Some(600));
}

#[test]
fn flags_accumulate() {
    let mut h = valid();
    assert!(!h.is_stale());

    h.set_flags(header_flags::STALE);
    h.set_flags(header_flags::BID_EMPTY);
    assert!(h.is_stale());
    assert!(h.has(header_flags::BID_EMPTY));
    assert!(!h.has(header_flags::ASK_EMPTY));

    // `has` requires *every* bit of the mask.
    assert!(h.has(header_flags::STALE | header_flags::BID_EMPTY));
    assert!(!h.has(header_flags::STALE | header_flags::ASK_EMPTY));
}

#[test]
fn producer_seq_is_the_ordering_authority_not_the_exchange_sequence() {
    // The exchange distribution sequence stays inside the handler; KRX blanks
    // it on some channels. Only the ring sequence lives in the header.
    let mut h = valid();
    h.set_producer_seq(42);
    assert_eq!(h.producer_seq, 42);
}
