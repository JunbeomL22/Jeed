//! `jeed_krx::decode::derivative::trade_quote` — `G7` 파생 체결 + 우선호가.

mod common;

use common::{G7, Level, RECV_NS, VENUE_NS, kospi200_book};
use jeed_krx::KrxError;
use jeed_krx::decode::derivative::trade_quote::{FIVE_DEEP, TEN_DEEP, depth_for};
use jeed_wire::{Scale, WireKind, WireRecord, header_flags, trade_flags, trade_kind};

fn decode(msg: &[u8]) -> WireRecord {
    let mut out = WireRecord::zeroed();
    FIVE_DEEP.decode(msg, RECV_NS, &mut out).expect("decodes");
    out
}

#[test]
fn the_five_deep_and_ten_deep_variants_are_the_lengths_the_spec_defines() {
    assert_eq!(FIVE_DEEP.message_len(), 431); // IFMSRPD0037
    assert_eq!(TEN_DEEP.message_len(), 661); // IFMSRPD0038
}

#[test]
fn the_print_and_the_book_it_left_arrive_as_one_record() {
    // This is why G7 is the channel Jeed is built around: a separate trade and
    // quote can be reordered or half-lost, and these cannot.
    let rec = decode(&G7::kospi200(kospi200_book()).build());

    assert_eq!(rec.validate(), Ok(()));
    assert_eq!(rec.kind(), Ok(WireKind::TradeQuote));
    let tq = rec.trade_quote().unwrap();

    assert_eq!(tq.trade.price, 93700);
    assert_eq!(tq.trade.qty, 3);
    assert_eq!(tq.quote.ask[0].price, 93705);
    assert_eq!(tq.quote.bid[0].price, 93695);
    assert_eq!(rec.header.depth, 5);
    assert_eq!(rec.header.price_scale(), Ok(Scale::S2));
}

#[test]
fn the_dynamic_price_limits_come_from_the_right_nine_bytes() {
    // [154:163] and [163:172]. Reading the spec's offset column as a *start*
    // gives [163:172]/[172:181], which parses fine and yields an upper limit
    // below the lower one — the failure that made layouts.md generated.
    let rec = decode(&G7::kospi200(kospi200_book()).build());
    let t = rec.trade_quote().unwrap().trade;

    assert_eq!(t.dyn_limits(), Some((94635, 92765)));
    let (upper, lower) = t.dyn_limits().unwrap();
    assert!(upper > lower, "an upper limit below the lower one means the offsets slipped");
    assert!(upper > t.price && t.price > lower, "the print sits inside its own band");
}

#[test]
fn an_instrument_outside_the_regime_is_not_given_a_zero_band() {
    // Far-month futures and spreads carry `000000.00`, not blanks. "Zero" and
    // "not applicable" are indistinguishable in the message, so the conclusion
    // rides in a flag and the consumer never has to make it again.
    let mut msg = G7::kospi200(kospi200_book());
    msg.dyn_limits = ("000000.00", "000000.00");
    let rec = decode(&msg.build());
    let t = rec.trade_quote().unwrap().trade;

    assert_eq!(t.dyn_limits(), None);
    assert_eq!(t.trade_flags & trade_flags::DYN_LIMIT_VALID, 0);
}

#[test]
fn a_half_present_band_is_not_half_believed() {
    let mut msg = G7::kospi200(kospi200_book());
    msg.dyn_limits = ("000946.35", "000000.00");
    assert_eq!(decode(&msg.build()).trade_quote().unwrap().trade.dyn_limits(), None);
}

#[test]
fn the_cumulative_quantity_is_carried_and_marked() {
    let rec = decode(&G7::kospi200(kospi200_book()).build());
    assert_eq!(rec.trade_quote().unwrap().trade.cumulative_qty(), Some(166_478));
}

#[test]
fn the_aggressor_code_maps_to_the_wire_encoding() {
    for (code, expected) in
        [(b'1', trade_kind::SELL), (b'2', trade_kind::BUY), (b'0', trade_kind::UNKNOWN)]
    {
        let mut msg = G7::kospi200(kospi200_book());
        msg.aggressor = code;
        assert_eq!(decode(&msg.build()).trade_quote().unwrap().trade.trade_kind, expected);
    }
}

#[test]
fn an_auction_cross_has_no_aggressor_but_the_field_was_still_there() {
    // 최종매도매수구분코드 is a space for 단일가체결. That is UNKNOWN ("the field
    // said nothing"), never NONE ("this channel has no such field").
    let mut msg = G7::kospi200(kospi200_book());
    msg.aggressor = b' ';
    let t = decode(&msg.build()).trade_quote().unwrap().trade;

    assert_eq!(t.trade_kind, trade_kind::UNKNOWN);
    assert_ne!(t.trade_kind, trade_kind::NONE);
}

#[test]
fn the_exchange_timestamp_is_assembled_and_marked_valid() {
    let rec = decode(&G7::kospi200(kospi200_book()).build());
    assert!(rec.header.has(header_flags::VENUE_TIME_VALID));
    assert_eq!(rec.header.venue_ns, VENUE_NS);
    assert_eq!(rec.header.venue_age_ns(), Some(RECV_NS - VENUE_NS));
}

#[test]
fn a_print_with_no_resting_book_flags_both_sides() {
    let rec = decode(&G7::kospi200(vec![Level::EMPTY; 5]).build());
    assert!(rec.header.has(header_flags::BID_EMPTY));
    assert!(rec.header.has(header_flags::ASK_EMPTY));
    assert_eq!(rec.header.depth, 0);
    assert_eq!(rec.trade_quote().unwrap().trade.price, 93700, "the print is still there");
}

#[test]
fn a_spread_quote_may_be_negative() {
    // The one case KRX sends a '-': a calendar spread can trade below zero.
    let mut msg = G7::kospi200(kospi200_book());
    msg.price = "-00000.85";
    assert_eq!(decode(&msg.build()).trade_quote().unwrap().trade.price, -85);
}

#[test]
fn a_corrupt_field_leaves_the_output_untouched() {
    let mut msg = G7::kospi200(kospi200_book()).build();
    msg[154] = b'X'; // sign byte of 동적상한가

    let mut out = WireRecord::zeroed();
    assert_eq!(
        FIVE_DEEP.decode(&msg, RECV_NS, &mut out),
        Err(KrxError::Sign { at: 154, found: b'X' })
    );
    assert_eq!(out, WireRecord::zeroed(), "no partial update");
}

#[test]
fn a_message_missing_its_end_keyword_is_refused() {
    let mut msg = G7::kospi200(kospi200_book()).build();
    *msg.last_mut().unwrap() = b'0';

    let mut out = WireRecord::zeroed();
    assert_eq!(
        FIVE_DEEP.decode(&msg, RECV_NS, &mut out),
        Err(KrxError::EndKeyword { found: b'0' })
    );
}

#[test]
fn ten_deep_is_the_same_decoder_with_a_deeper_book() {
    let mut levels = kospi200_book();
    levels.extend(kospi200_book());
    let mut msg = G7::kospi200(levels);
    msg.header.trcode = "G705F"; // 주식옵션 — the real ten-deep case
    msg.price = "000074100";
    msg.dyn_limits = ("000074800", "000073400");

    let mut out = WireRecord::zeroed();
    TEN_DEEP.decode(&msg.build(), RECV_NS, &mut out).expect("decodes");
    assert_eq!(out.header.depth, 10);
    assert_eq!(out.trade_quote().unwrap().trade.price, 74100);
    assert_eq!(out.trade_quote().unwrap().trade.dyn_limits(), Some((74800, 73400)));
}

#[test]
fn the_depth_rule_is_the_same_one_the_quote_decoder_uses() {
    use jeed_krx::TrCode as T;
    assert_eq!(depth_for(T::new(*b"G705F")), Some(10), "주식옵션");
    assert_eq!(depth_for(T::new(*b"G718F")), Some(10), "개별주식 위클리옵션");
    assert_eq!(depth_for(T::new(*b"G704F")), Some(5), "주식선물 — truncated to five");
    assert_eq!(depth_for(T::new(*b"G701F")), Some(5));
    assert_eq!(depth_for(T::new(*b"G717F")), Some(5), "코스닥150 위클리옵션");
    assert_eq!(depth_for(T::new(*b"B601F")), None, "not a G7");
}
