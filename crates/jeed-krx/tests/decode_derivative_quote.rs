//! `jeed_krx::decode::derivative::quote` — `B6` 파생 우선호가.

mod common;

use common::{B6, Level, RECV_NS, VENUE_NS, kospi200_book};
use jeed_krx::KrxError;
use jeed_krx::decode::derivative::quote::{FIVE_DEEP, TEN_DEEP, depth_for};
use jeed_wire::{Scale, Venue, WireKind, WireRecord, header_flags, quote_ext};

fn decode(msg: &[u8]) -> WireRecord {
    let mut out = WireRecord::zeroed();
    FIVE_DEEP.decode(msg, RECV_NS, &mut out).expect("decodes");
    out
}

#[test]
fn the_five_deep_and_ten_deep_variants_are_the_lengths_the_spec_defines() {
    assert_eq!(FIVE_DEEP.message_len(), 324); // IFMSRPD0034
    assert_eq!(TEN_DEEP.message_len(), 554); // IFMSRPD0035
}

#[test]
fn a_kospi200_book_round_trips_into_a_quote_record() {
    let rec = decode(&B6::kospi200(kospi200_book()).build());

    assert_eq!(rec.validate(), Ok(()));
    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.isin, *b"KR4101V90009");
    assert_eq!(rec.header.venue, Venue::Krx.as_u8());
    assert_eq!(rec.header.depth, 5);

    let q = rec.quote().unwrap();
    assert_eq!(q.ask[0].price, 93705);
    assert_eq!(q.bid[0].price, 93695);
    assert_eq!(q.ask[0].qty, 10);
    assert_eq!(q.bid[0].qty, 8);
    assert_eq!(q.ask[4].price, 93725);
    assert_eq!(q.bid[4].qty, 77);
}

#[test]
fn the_price_scale_comes_off_the_message_not_a_table() {
    // KOSPI200 futures spell prices [sign][5].[2]; the record says S2 and the
    // integer is the price times 100.
    let rec = decode(&B6::kospi200(kospi200_book()).build());
    assert_eq!(rec.header.price_scale(), Ok(Scale::S2));
    assert_eq!(rec.header.qty_scale(), Ok(Scale::S0));
    assert_eq!(rec.quote().unwrap().ask[0].price, 93705);
}

#[test]
fn a_single_stock_future_has_no_decimal_point_and_says_so() {
    // Same nine bytes, same channel shape, different instrument: [sign][8].
    let levels = vec![
        Level {
            ask_price: "000074100",
            bid_price: "000074000",
            ask_qty: 12,
            bid_qty: 9,
            ask_count: 2,
            bid_count: 1,
        },
        Level::EMPTY,
        Level::EMPTY,
        Level::EMPTY,
        Level::EMPTY,
    ];
    let mut msg = B6::kospi200(levels);
    msg.header.trcode = "B604F"; // 주식선물 — five-deep despite the ten-deep book
    let rec = decode(&msg.build());

    assert_eq!(rec.header.price_scale(), Ok(Scale::S0));
    assert_eq!(rec.quote().unwrap().ask[0].price, 74100);
}

#[test]
fn order_counts_are_declared_meaningful_on_derivative_channels() {
    let rec = decode(&B6::kospi200(kospi200_book()).build());
    let q = rec.quote().unwrap();
    assert!(q.has_order_counts());
    assert_eq!(q.ask[0].order_count, 3);
    assert_eq!(q.bid[2].order_count, 12);
}

#[test]
fn depth_is_counted_not_assumed() {
    // Levels past the end of the book arrive zero-filled, so a five-deep
    // channel carrying two resting levels reports two.
    let mut levels = kospi200_book();
    levels[2] = Level::EMPTY;
    levels[3] = Level::EMPTY;
    levels[4] = Level::EMPTY;
    let rec = decode(&B6::kospi200(levels).build());

    assert_eq!(rec.header.depth, 2);
    assert_eq!(rec.quote().unwrap().asks(rec.header.depth).len(), 2);
}

#[test]
fn an_empty_side_is_flagged_rather_than_left_to_the_consumer_to_notice() {
    let levels: Vec<Level> = kospi200_book()
        .into_iter()
        .map(|mut l| {
            l.bid_qty = 0;
            l.bid_count = 0;
            l
        })
        .collect();
    let rec = decode(&B6::kospi200(levels).build());

    assert!(rec.header.has(header_flags::BID_EMPTY));
    assert!(!rec.header.has(header_flags::ASK_EMPTY));
    assert_eq!(rec.header.depth, 5, "the ask side still has five levels");
}

#[test]
fn an_entirely_empty_book_flags_both_sides() {
    let rec = decode(&B6::kospi200(vec![Level::EMPTY; 5]).build());
    assert!(rec.header.has(header_flags::BID_EMPTY));
    assert!(rec.header.has(header_flags::ASK_EMPTY));
    assert_eq!(rec.header.depth, 0);
    assert_eq!(rec.header.price_scale(), Ok(Scale::S2), "zeros still spell the scale");
}

#[test]
fn the_exchange_timestamp_is_assembled_and_marked_valid() {
    let rec = decode(&B6::kospi200(kospi200_book()).build());
    assert!(rec.header.has(header_flags::VENUE_TIME_VALID));
    assert_eq!(rec.header.venue_ns, VENUE_NS);
    assert_eq!(rec.header.recv_ns, RECV_NS);
    assert_eq!(rec.header.venue_age_ns(), Some(RECV_NS - VENUE_NS));
}

#[test]
fn a_blank_timestamp_is_not_marked_valid() {
    // Blank means "not measurable". Marking a zero valid would let the consumer
    // age the book against the epoch and call it stale forever.
    let mut msg = B6::kospi200(kospi200_book());
    msg.header.time = "            ";
    let rec = decode(&msg.build());

    assert!(!rec.header.has(header_flags::VENUE_TIME_VALID));
    assert_eq!(rec.header.venue_time(), None);
}

#[test]
fn a_blank_sequence_number_does_not_stop_the_decode() {
    // B606F sent nothing but blanks until 2026-04-01 and V103F still does. The
    // exchange sequence never reaches the wire anyway — ordering authority is
    // producer_seq (feed_handler.md §7).
    let mut msg = B6::kospi200(kospi200_book());
    msg.header.sequence = None;
    let rec = decode(&msg.build());
    assert_eq!(rec.kind(), Ok(WireKind::Quote));
}

#[test]
fn the_auction_indicative_price_rides_in_the_spare_ext_word() {
    let mut msg = B6::kospi200(kospi200_book());
    msg.expected_price = "000937.00";
    let rec = decode(&msg.build());

    let q = rec.quote().unwrap();
    assert_eq!(q.quote_ext_kind, quote_ext::EXPECTED_PRICE);
    assert_eq!(q.quote_ext, 93700);
}

#[test]
fn no_auction_running_means_no_indicative_price() {
    let rec = decode(&B6::kospi200(kospi200_book()).build());
    assert_eq!(rec.quote().unwrap().quote_ext_kind, quote_ext::NONE);
}

#[test]
fn a_message_of_the_wrong_length_leaves_the_output_untouched() {
    // The reaction to a bad message is to drop the slot, so the decoder must
    // not have written anything into it (CLAUDE.md).
    let mut msg = B6::kospi200(kospi200_book()).build();
    msg.truncate(300);

    let mut out = WireRecord::zeroed();
    assert_eq!(
        FIVE_DEEP.decode(&msg, RECV_NS, &mut out),
        Err(KrxError::Length { expected: 324, actual: 300 })
    );
    assert_eq!(out, WireRecord::zeroed());
}

#[test]
fn a_corrupt_field_leaves_the_output_untouched() {
    let mut msg = B6::kospi200(kospi200_book()).build();
    msg[47] = b'X'; // sign byte of the first ask price

    let mut out = WireRecord::zeroed();
    assert_eq!(
        FIVE_DEEP.decode(&msg, RECV_NS, &mut out),
        Err(KrxError::Sign { at: 47, found: b'X' })
    );
    assert_eq!(out, WireRecord::zeroed(), "no partial update");
}

#[test]
fn ten_deep_is_the_same_decoder_with_a_deeper_book() {
    let mut levels = kospi200_book();
    levels.extend(kospi200_book());
    let mut msg = B6::kospi200(levels);
    msg.header.trcode = "B605F"; // 주식옵션 — the real ten-deep case

    let mut out = WireRecord::zeroed();
    TEN_DEEP.decode(&msg.build(), RECV_NS, &mut out).expect("decodes");
    assert_eq!(out.header.depth, 10);
    assert_eq!(out.quote().unwrap().ask[9].price, 93725);
}

#[test]
fn only_single_stock_options_are_ten_deep() {
    use jeed_krx::TrCode as T;
    // Ten-deep: 주식옵션 05F, 개별주식 위클리옵션 18F.
    assert_eq!(depth_for(T::new(*b"B605F")), Some(10));
    assert_eq!(depth_for(T::new(*b"B618F")), Some(10));
    // Everything else, including the new weekly index options.
    for code in [b"B601F", b"B602F", b"B603F", b"B606F", b"B611F", b"B616F", b"B617F"] {
        assert_eq!(depth_for(T::new(*code)), Some(5), "{}", T::new(*code));
    }
    assert_eq!(depth_for(T::new(*b"B601S")), None, "not a derivative");
    assert_eq!(depth_for(T::new(*b"G701F")), None, "not a B6");
}

#[test]
fn single_stock_futures_are_truncated_to_five_levels() {
    // The trap. 주식선물 is a ten-deep product, but the feed sends five and
    // nothing downstream uses more (CLAUDE.md). The distribution standard files
    // B604F under the ten-deep interface alone and the channel standard lists
    // it under both, so the documents cannot settle it — this does.
    //
    // It is also the highest-volume code on the line (26.9M in one day), so
    // reading it ten-deep loses half the feed.
    assert_eq!(depth_for(jeed_krx::TrCode::new(*b"B604F")), Some(5));
    assert_eq!(FIVE_DEEP.message_len(), 324);
}
