//! `jeed_wire::record` — the record, its byte views, and kind gating.

use jeed_wire::{
    HeartbeatPayload, QuotePayload, RecordHeader, Scale, TradePayload, TradeQuotePayload,
    WIRE_ALIGN, WIRE_HEADER_LEN, WIRE_PAYLOAD_LEN, WIRE_RECORD_LEN, WireError, WireKind, WireLevel,
    WireRecord, Venue, level_ext, trade_kind,
};

fn header(kind: WireKind) -> RecordHeader {
    let mut h = RecordHeader::new(kind, Venue::Krx, jeed_wire::symbol_from_bytes(b"KR4A01690002").unwrap(), 1_785_455_100_003_679_000);
    h.set_scales(Scale::S2, Scale::S0);
    h
}

fn quote_record() -> WireRecord {
    let mut q = QuotePayload::default();
    q.set_ask(0, WireLevel::with_count(93705, 10, 3));
    q.set_bid(0, WireLevel::with_count(93695, 8, 2));
    q.with_order_counts();

    let mut h = header(WireKind::Quote);
    h.set_depth(1);
    h.set_venue_time(1_785_455_100_000_000_000);
    WireRecord::new_quote(h, q)
}

#[test]
fn layout_is_ten_whole_cache_lines() {
    assert_eq!(size_of::<WireRecord>(), WIRE_RECORD_LEN);
    assert_eq!(size_of::<WireRecord>(), 640);
    assert_eq!(align_of::<WireRecord>(), WIRE_ALIGN);
    assert_eq!(WIRE_RECORD_LEN % WIRE_ALIGN, 0);
    assert_eq!(WIRE_RECORD_LEN / WIRE_ALIGN, 10);
    // 64 + 544 = 608, which is not a whole number of cache lines, so the
    // record carries 32 bytes of explicit tail. Explicit, because implicit
    // padding would be uninitialised bytes going onto the wire.
    //
    // The header's widening from 48 to 64 (v1 → v2) came out of this tail,
    // not out of the record: 640 held then and holds now.
    assert_eq!(WIRE_HEADER_LEN + WIRE_PAYLOAD_LEN, 608);
    assert_eq!(WIRE_RECORD_LEN - 608, 32);
}

#[test]
fn zeroed_record_is_invalid() {
    let rec = WireRecord::zeroed();
    assert_eq!(rec.validate(), Err(WireError::Kind { found: 0 }));
    assert_eq!(rec.as_bytes(), &[0u8; WIRE_RECORD_LEN]);
}

#[test]
fn bytes_round_trip_through_a_slice() {
    let rec = quote_record();
    let copy = WireRecord::from_bytes(rec.as_bytes()).expect("valid record");
    assert_eq!(copy, rec);
    assert_eq!(copy.quote().unwrap().ask[0].price, 93705);
}

#[test]
fn round_trip_survives_an_unaligned_slice() {
    // `from_bytes` copies, so a byte stream that does not land on a 64-byte
    // boundary is still readable.
    let rec = quote_record();
    let mut buf = vec![0u8; WIRE_RECORD_LEN + 1];
    buf[1..].copy_from_slice(rec.as_bytes());

    let copy = WireRecord::from_bytes(&buf[1..]).expect("unaligned copy is fine");
    assert_eq!(copy, rec);
}

#[test]
fn wrong_length_is_rejected() {
    let rec = quote_record();
    let bytes = rec.as_bytes();
    assert_eq!(
        WireRecord::from_bytes(&bytes[..WIRE_RECORD_LEN - 1]),
        Err(WireError::Length { expected: WIRE_RECORD_LEN, actual: WIRE_RECORD_LEN - 1 })
    );
}

#[test]
fn in_place_borrow_requires_alignment() {
    let rec = quote_record();

    let aligned = WireRecord::ref_from_bytes(rec.as_bytes()).expect("record is 64-byte aligned");
    assert_eq!(aligned.header.symbol_bytes(), b"KR4A01690002");

    // The first offset into `buf` that lands eight bytes past a 64-byte
    // boundary, whatever the allocator handed us.
    let mut buf = vec![0u8; WIRE_RECORD_LEN + WIRE_ALIGN];
    let off = (8 + WIRE_ALIGN - (buf.as_ptr() as usize % WIRE_ALIGN)) % WIRE_ALIGN;
    assert_eq!((buf.as_ptr() as usize + off) % WIRE_ALIGN, 8);
    buf[off..off + WIRE_RECORD_LEN].copy_from_slice(rec.as_bytes());
    assert_eq!(
        WireRecord::ref_from_bytes(&buf[off..off + WIRE_RECORD_LEN]),
        Err(WireError::Alignment { required: WIRE_ALIGN })
    );
}

#[test]
fn typed_access_is_gated_by_the_header_kind() {
    let rec = quote_record();
    assert!(rec.quote().is_ok());
    assert_eq!(
        rec.trade().unwrap_err(),
        WireError::KindMismatch { expected: WireKind::Trade, found: WireKind::Quote }
    );
    assert_eq!(
        rec.heartbeat().unwrap_err(),
        WireError::KindMismatch { expected: WireKind::Heartbeat, found: WireKind::Quote }
    );
}

#[test]
fn installing_a_payload_zeroes_the_rest_of_the_area() {
    // Otherwise a short payload would leave the tail of a previous long one
    // visible to anyone reading raw bytes.
    let mut rec = quote_record();
    assert_ne!(rec.payload_bytes()[..8], [0u8; 8]);

    rec.set_heartbeat(HeartbeatPayload { received: 7, forwarded: 5 });
    assert_eq!(rec.kind(), Ok(WireKind::Heartbeat));
    assert_eq!(rec.heartbeat().unwrap().received, 7);
    assert_eq!(
        &rec.payload_bytes()[16..],
        &[0u8; WIRE_PAYLOAD_LEN - 16][..],
        "everything past the 16-byte heartbeat must be zero"
    );
}

#[test]
fn trade_quote_keeps_the_print_and_the_book_together() {
    let mut trade = TradePayload::new(93700, 2);
    trade.with_cumulative_qty(132).with_kind(trade_kind::BUY);
    let mut quote = QuotePayload::default();
    quote.set_ask(0, WireLevel::new(93705, 10));
    quote.set_bid(0, WireLevel::new(93695, 8));
    let tq = TradeQuotePayload { trade, quote };

    let mut h = header(WireKind::TradeQuote);
    h.set_depth(1);
    let rec = WireRecord::new_trade_quote(h, tq);

    assert_eq!(rec.validate(), Ok(()));
    let got = rec.trade_quote().unwrap();
    assert_eq!(got.trade.price, 93700);
    assert_eq!(got.trade.cumulative_qty(), Some(132));
    assert_eq!(got.quote.bid[0].qty, 8);
}

#[test]
fn unknown_payload_encodings_are_rejected() {
    let mut rec = quote_record();
    rec.quote_mut().unwrap().level_ext_kind = 99;
    assert_eq!(rec.validate(), Err(WireError::LevelExtKind { found: 99 }));

    let mut rec = quote_record();
    rec.quote_mut().unwrap().quote_ext_kind = 42;
    assert_eq!(rec.validate(), Err(WireError::QuoteExtKind { found: 42 }));

    let mut rec = WireRecord::new_trade(header(WireKind::Trade), TradePayload::new(100, 1));
    rec.trade_mut().unwrap().trade_kind = 7;
    assert_eq!(rec.validate(), Err(WireError::TradeKind { found: 7 }));
}

#[test]
fn validation_runs_on_the_way_in_from_bytes() {
    let mut rec = quote_record();
    rec.quote_mut().unwrap().with_level_ext(99);
    let bytes = *rec.as_bytes();

    // The record is corrupt, so it must not come back out as a value.
    assert_eq!(WireRecord::from_bytes(&bytes), Err(WireError::LevelExtKind { found: 99 }));
    assert_eq!(WireRecord::ref_from_bytes(&bytes), Err(WireError::LevelExtKind { found: 99 }));
}

#[test]
fn known_encodings_pass_validation() {
    let mut rec = quote_record();
    rec.quote_mut().unwrap().with_level_ext(level_ext::LP_QUANTITY);
    assert_eq!(rec.validate(), Ok(()));
}

#[test]
fn debug_names_the_payload_it_actually_carries() {
    let rec = quote_record();
    let s = format!("{rec:?}");
    assert!(s.contains("quote"), "{s}");
    assert!(!s.contains("heartbeat"), "{s}");
}

#[test]
fn snapshot_delta_is_no_longer_a_reserved_kind() {
    let mut d = jeed_wire::SnapshotDeltaPayload {
        first_update_id: 157,
        final_update_id: 160,
        bid_count: 1,
        ask_count: 1,
        ..Default::default()
    };
    d.levels[0] = jeed_wire::WireDeltaLevel { price: 2_400, qty: 10 };
    d.levels[1] = jeed_wire::WireDeltaLevel { price: 2_600, qty: 0 };

    let rec = WireRecord::new_snapshot_delta(header(WireKind::SnapshotDelta), d);
    assert_eq!(rec.validate(), Ok(()));
    assert_eq!(rec.snapshot_delta().unwrap().final_update_id, 160);
    assert_eq!(rec.trade(), Err(WireError::KindMismatch {
        expected: WireKind::Trade,
        found: WireKind::SnapshotDelta,
    }));
}

#[test]
fn a_delta_claiming_more_levels_than_it_holds_is_rejected() {
    // Off-wire defence: the decoder refuses to build one, but a record read
    // back from a segment written by another build must not be trusted.
    let d = jeed_wire::SnapshotDeltaPayload { bid_count: 20, ask_count: 20, ..Default::default() };
    let rec = WireRecord::new_snapshot_delta(header(WireKind::SnapshotDelta), d);
    assert_eq!(
        rec.validate(),
        Err(WireError::DeltaLevelCount { bid: 20, ask: 20, max: 32 })
    );
}

#[test]
fn snapshot_delta_is_the_one_kind_that_does_not_heal_itself() {
    // Everything else replaces its predecessor, so a ring drop costs an
    // instant. A dropped delta leaves the book wrong until a resync.
    assert!(!WireKind::SnapshotDelta.is_self_healing());
    for k in [WireKind::Quote, WireKind::Trade, WireKind::TradeQuote] {
        assert!(k.is_self_healing());
    }
}
