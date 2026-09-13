//! `jeed_krx::decode::etf::quote` — `B7` ETF·ELW·ETN 우선호가.

use crate::common::{EtfB7, RECV_NS, StockB6};
use jeed_krx::decode::etf::quote::{DECODER, MESSAGE_LEN, handles};
use jeed_krx::decode::stock;
use jeed_wire::{Scale, WireKind, WireRecord, level_ext, level_flags, quote_ext};

fn decode(msg: &[u8]) -> WireRecord {
    let mut out = WireRecord::zeroed();
    DECODER.decode(msg, RECV_NS, &mut out).expect("decodes");
    out
}

#[test]
fn the_message_is_the_length_the_spec_defines() {
    assert_eq!(MESSAGE_LEN, 830); // IFMSRPD0003
    assert_eq!(DECODER.depth(), 10);
}

#[test]
fn a_ten_deep_book_round_trips_into_a_quote_record() {
    let rec = decode(&EtfB7::kodex200().build());

    assert_eq!(rec.validate(), Ok(()));
    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    assert_eq!(rec.header.isin, *b"KR7069500007");
    assert_eq!(rec.header.depth, 10);
    assert_eq!(rec.header.price_scale(), Ok(Scale::S0));

    let q = rec.quote().unwrap();
    assert_eq!(q.ask[0].price, 74100);
    assert_eq!(q.bid[0].price, 74000);
    assert_eq!(q.ask[9].price, 75000);
}

#[test]
fn the_lp_quantity_is_part_of_the_level_not_extra_to_it() {
    // 매도1단계LP우선호가잔량 is the LP's share of the size already reported at
    // that level. If it were added to `qty` the book would double-count the LP;
    // if it were dropped, the part of the level most likely to still be there
    // would be invisible.
    let rec = decode(&EtfB7::kodex200().build());
    let q = rec.quote().unwrap();

    assert_eq!(q.level_ext_kind, level_ext::LP_QUANTITY);
    assert_eq!(q.ask[0].qty, 100, "the whole resting size");
    assert_eq!(q.ask[0].ext, 10, "of which the LP's");
    assert!(u64::from(q.ask[0].ext) < q.ask[0].qty);
    assert_eq!(q.bid[0].ext, 20);
    assert_eq!(q.ask[9].ext, 19);
}

#[test]
fn a_consumer_ignoring_the_ext_word_still_sees_a_correct_book() {
    // The same book on the two forms has to agree on price and size; only the
    // LP breakdown is extra. B6 and B7 spell their levels differently (46 vs 70
    // bytes), so this is a real check on both layouts, not a tautology.
    let from_b7 = decode(&EtfB7::kodex200().build());

    let mut from_b6 = WireRecord::zeroed();
    stock::quote::DECODER
        .decode(&StockB6::samsung().build(), RECV_NS, &mut from_b6)
        .expect("decodes");

    let (a, b) = (from_b7.quote().unwrap(), from_b6.quote().unwrap());
    for level in 0..10 {
        assert_eq!(a.ask[level].price, b.ask[level].price, "ask {level}");
        assert_eq!(a.ask[level].qty, b.ask[level].qty, "ask {level}");
        assert_eq!(a.bid[level].price, b.bid[level].price, "bid {level}");
        assert_eq!(a.bid[level].qty, b.bid[level].qty, "bid {level}");
    }
    assert_eq!(from_b7.header.depth, from_b6.header.depth);
}

#[test]
fn order_counts_are_not_claimed_here_either() {
    assert!(!decode(&EtfB7::kodex200().build()).quote().unwrap().has_order_counts());
    assert_eq!(
        decode(&EtfB7::kodex200().build()).quote().unwrap().level_flags
            & level_flags::ORDER_COUNT_VALID,
        0
    );
}

#[test]
fn the_auction_indicative_price_rides_in_the_spare_ext_word() {
    let mut msg = EtfB7::kodex200();
    msg.tail.expected_price = "00000074050";
    let rec = decode(&msg.build());

    let q = rec.quote().unwrap();
    assert_eq!(q.quote_ext_kind, quote_ext::EXPECTED_PRICE);
    assert_eq!(q.quote_ext, 74050);
}

#[test]
fn a_corrupt_field_leaves_the_output_untouched() {
    let mut msg = EtfB7::kodex200().build();
    msg[47] = b'X';

    let mut out = WireRecord::zeroed();
    assert!(DECODER.decode(&msg, RECV_NS, &mut out).is_err());
    assert_eq!(out, WireRecord::zeroed(), "no partial update");
}

#[test]
fn the_four_lp_product_groups_are_claimed_and_nothing_else() {
    use jeed_krx::TrCode as T;
    for code in [b"B702S", b"B703S", b"B704S", b"B705S"] {
        assert!(handles(T::new(*code)), "{}", T::new(*code));
    }
    // 주식 has no liquidity provider, so B701S is not a message KRX sends.
    assert!(!handles(T::new(*b"B701S")));
    assert!(!handles(T::new(*b"B701Q")));
    // Nor is B603S: the split by LP is clean in both directions.
    assert!(!stock::quote::handles(T::new(*b"B603S")));
}
