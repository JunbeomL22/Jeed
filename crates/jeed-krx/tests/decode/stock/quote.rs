//! `jeed_krx::decode::stock::quote` — `B6` 주식 우선호가.

use crate::common::{EquityLevel, RECV_NS, StockB6, VENUE_NS, samsung_book};
use jeed_krx::KrxError;
use jeed_krx::decode::stock::quote::{DECODER, MESSAGE_LEN, handles};
use jeed_wire::{Scale, Venue, WireKind, WireRecord, header_flags, level_flags, quote_ext};

fn decode(msg: &[u8]) -> WireRecord {
    let mut out = WireRecord::zeroed();
    DECODER.decode(msg, RECV_NS, &mut out).expect("decodes");
    out
}

#[test]
fn the_message_is_the_length_the_spec_defines() {
    assert_eq!(MESSAGE_LEN, 590); // IFMSRPD0002
    assert_eq!(DECODER.depth(), 10);
}

#[test]
fn a_ten_deep_book_round_trips_into_a_quote_record() {
    let rec = decode(&StockB6::samsung().build());

    assert_eq!(rec.validate(), Ok(()));
    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.isin, *b"KR7005930003");
    assert_eq!(rec.header.venue, Venue::Krx.as_u8());
    assert_eq!(rec.header.depth, 10);

    let q = rec.quote().unwrap();
    assert_eq!(q.ask[0].price, 74100);
    assert_eq!(q.bid[0].price, 74000);
    assert_eq!(q.ask[0].qty, 100);
    assert_eq!(q.bid[0].qty, 200);
    assert_eq!(q.ask[9].price, 75000);
    assert_eq!(q.bid[9].price, 73100);
}

#[test]
fn a_share_price_has_no_decimal_places() {
    // 증권 prices are eleven bytes with no point at all: [부호][미사용][9].
    let rec = decode(&StockB6::samsung().build());
    assert_eq!(rec.header.price_scale(), Ok(Scale::S0));
    assert_eq!(rec.header.qty_scale(), Ok(Scale::S0));
}

#[test]
fn order_counts_are_not_claimed_because_the_channel_has_none() {
    // The derivative channels carry 주문건수 per level and 증권 does not. A zero
    // count with the flag set would read as "no orders rest here", which is a
    // claim this message never made.
    let rec = decode(&StockB6::samsung().build());
    let q = rec.quote().unwrap();
    assert!(!q.has_order_counts());
    assert_eq!(q.level_flags & level_flags::ORDER_COUNT_VALID, 0);
    assert_eq!(q.ask[0].order_count, 0);
}

#[test]
fn there_is_no_lp_quantity_on_this_form() {
    // B6 is the MM/LP호가 제외 form. Leaving level_ext unset is what tells the
    // consumer the ext word means nothing here, rather than meaning zero LP.
    let rec = decode(&StockB6::samsung().build());
    assert_eq!(rec.quote().unwrap().level_ext_kind, jeed_wire::level_ext::NONE);
}

#[test]
fn depth_is_counted_not_assumed() {
    let mut msg = StockB6::samsung();
    for level in msg.levels.iter_mut().skip(3) {
        *level = EquityLevel::EMPTY;
    }
    let rec = decode(&msg.build());
    assert_eq!(rec.header.depth, 3);
}

#[test]
fn an_empty_side_is_flagged() {
    let mut msg = StockB6::samsung();
    for level in msg.levels.iter_mut() {
        level.bid_qty = 0;
    }
    let rec = decode(&msg.build());

    assert!(rec.header.has(header_flags::BID_EMPTY));
    assert!(!rec.header.has(header_flags::ASK_EMPTY));
    assert_eq!(rec.header.depth, 10, "the ask side is still ten deep");
}

#[test]
fn the_auction_indicative_price_rides_in_the_spare_ext_word() {
    let mut msg = StockB6::samsung();
    msg.tail.expected_price = "00000074050";
    let rec = decode(&msg.build());

    let q = rec.quote().unwrap();
    assert_eq!(q.quote_ext_kind, quote_ext::EXPECTED_PRICE);
    assert_eq!(q.quote_ext, 74050);
}

#[test]
fn no_auction_running_means_no_indicative_price() {
    assert_eq!(
        decode(&StockB6::samsung().build()).quote().unwrap().quote_ext_kind,
        quote_ext::NONE
    );
}

#[test]
fn the_exchange_timestamp_is_assembled_and_marked_valid() {
    let rec = decode(&StockB6::samsung().build());
    assert!(rec.header.has(header_flags::VENUE_TIME_VALID));
    assert_eq!(rec.header.venue_ns, VENUE_NS);
}

#[test]
fn a_message_of_the_wrong_length_leaves_the_output_untouched() {
    let mut msg = StockB6::samsung().build();
    msg.truncate(580);

    let mut out = WireRecord::zeroed();
    assert_eq!(
        DECODER.decode(&msg, RECV_NS, &mut out),
        Err(KrxError::Length { expected: 590, actual: 580 })
    );
    assert_eq!(out, WireRecord::zeroed());
}

#[test]
fn a_corrupt_field_leaves_the_output_untouched() {
    let mut msg = StockB6::samsung().build();
    msg[47] = b'X'; // sign byte of the first ask price

    let mut out = WireRecord::zeroed();
    assert!(DECODER.decode(&msg, RECV_NS, &mut out).is_err());
    assert_eq!(out, WireRecord::zeroed(), "no partial update");
}

#[test]
fn only_the_three_stock_boards_are_claimed() {
    use jeed_krx::TrCode as T;
    for code in [b"B601S", b"B601Q", b"B601X"] {
        assert!(handles(T::new(*code)), "{}", T::new(*code));
    }
    // The LP products take B7 instead — B603S does not exist.
    for code in [b"B602S", b"B603S", b"B604S", b"B605S"] {
        assert!(!handles(T::new(*code)), "{}", T::new(*code));
    }
    assert!(!handles(T::new(*b"B601F")), "파생");
    assert!(!handles(T::new(*b"B601K")), "채권");
}

#[test]
fn the_book_is_ten_deep_with_no_shallower_variant() {
    // Unlike the derivative channels there is nothing to select here: 주식
    // 우선호가 is one interface at one depth, so there is no depth_for and no
    // way to apply the wrong layout.
    assert_eq!(samsung_book().len(), DECODER.depth());
}
