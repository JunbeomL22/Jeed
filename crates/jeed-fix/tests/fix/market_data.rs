//! Tests for `src/data/fix/market_data.rs` — `35=W` / `35=X` decoding.

use super::{
    wrap, DAY_NS, HEARTBEAT, OPEN_SNAPSHOT, SCALES, TRADE_INCREMENTAL, TWO_SIDED_SNAPSHOT,
};
use jeed_fix::{
    frame, parse_md_message, FixError, MdEntryType, MdMessage, MdUpdateAction, MsgType,
    MD_MAX_ENTRIES,
};
use jeed_wire::Scale;

fn parse(body: &str) -> Result<MdMessage, FixError> {
    let raw = wrap(body);
    let f = frame(&raw).expect("framing must succeed before decoding");
    parse_md_message(&f, SCALES)
}

#[test]
fn a_two_sided_snapshot_decodes_into_indexed_levels() {
    let msg = parse(TWO_SIDED_SNAPSHOT).unwrap();
    assert_eq!(msg.msg_type, MsgType::MarketDataSnapshot);
    assert!(msg.is_snapshot());
    assert_eq!(msg.msg_seq_num, 2);
    assert_eq!(msg.symbol.as_bytes(), b"USDKRW");
    assert_eq!(msg.req_id.as_bytes(), b"USDKRW-SMBS");
    assert_eq!(msg.declared_entries, 2);
    assert_eq!(msg.len(), 2);
    assert!(!msg.poss_dup);

    let bid = msg.entries()[0];
    assert_eq!(bid.entry_type, MdEntryType::Bid);
    assert_eq!(bid.price, 145_000, "1450.00 on S2");
    assert_eq!(bid.size, 5_000_000);
    assert_eq!(bid.level, 0);
    assert!(!bid.has_explicit_level, "the level is implied by position");
    assert_eq!(bid.action, MdUpdateAction::New, "a snapshot entry is new by construction");

    let ask = msg.entries()[1];
    assert_eq!(ask.entry_type, MdEntryType::Offer);
    assert_eq!(ask.price, 145_300);
    assert_eq!(ask.level, 0, "the first offer is level 0, not level 1");

    assert_eq!(msg.side_depth(MdEntryType::Bid), 1);
    assert_eq!(msg.side_depth(MdEntryType::Offer), 1);
    assert_eq!(msg.side_depth(MdEntryType::Trade), 0);
}

#[test]
fn venue_time_comes_from_272_273_never_from_sending_time() {
    // The capture splits the two on purpose: 52 is venue+5ms, 272/273 is the
    // venue's own clock. A decoder that reads 52 would report a book 5 ms in
    // the future and inflate every measured feed latency.
    let msg = parse(TWO_SIDED_SNAPSHOT).unwrap();
    let entry = msg.entries()[0];
    assert_eq!(entry.date_ns, DAY_NS);
    assert_eq!(entry.time_ns, 1_000_000_000);
    assert_eq!(entry.venue_time(), Some(DAY_NS + 1_000_000_000));
    assert_eq!(msg.venue_time(), Some(DAY_NS + 1_000_000_000));
    assert_eq!(msg.sending_time, DAY_NS + 1_005_000_000);
    assert_ne!(msg.venue_time(), Some(msg.sending_time));
}

#[test]
fn a_one_sided_open_keeps_the_missing_side_missing() {
    // 2026-02-02 09:00 KST: the source has exactly one row with a null bid.
    let msg = parse(OPEN_SNAPSHOT).unwrap();
    assert_eq!(msg.len(), 1);
    assert_eq!(msg.entries()[0].entry_type, MdEntryType::Offer);
    assert_eq!(msg.entries()[0].price, 145_300);
    assert_eq!(msg.side_depth(MdEntryType::Bid), 0, "no bid entry ⇒ no bid depth");
    assert_eq!(msg.side_depth(MdEntryType::Offer), 1);
}

#[test]
fn an_incremental_trade_decodes_with_its_own_symbol() {
    let msg = parse(TRADE_INCREMENTAL).unwrap();
    assert_eq!(msg.msg_type, MsgType::MarketDataIncremental);
    assert!(!msg.is_snapshot());
    assert_eq!(msg.len(), 1);
    assert!(msg.symbol.is_empty(), "35=X carries Symbol inside the group");

    let t = msg.entries()[0];
    assert_eq!(t.entry_type, MdEntryType::Trade);
    assert_eq!(t.action, MdUpdateAction::New);
    assert_eq!(t.price, 145_100);
    assert_eq!(t.size, 1_000_000);
    assert_eq!(msg.entry_symbol(&t), b"USDKRW");
    assert_eq!(t.venue_time(), Some(DAY_NS + 14_000_000_000));
}

#[test]
fn several_entries_split_on_the_right_delimiter_for_each_message_type() {
    // 35=X delimits on 279; two prints in one message must not merge.
    let msg = parse(
        "35=X|34=9|52=20260202-00:00:14.005|262=R|268=2|\
         279=0|269=2|55=USDKRW|270=1451.00|271=1000000|272=20260202|273=00:00:14.000|\
         279=0|269=2|55=USDKRW|270=1451.10|271=2000000|272=20260202|273=00:00:15.000",
    )
    .unwrap();
    assert_eq!(msg.len(), 2);
    assert_eq!(msg.entries()[0].price, 145_100);
    assert_eq!(msg.entries()[1].price, 145_110);
    assert_eq!(msg.entries()[1].size, 2_000_000);

    // 35=W delimits on 269; a 5-deep book keeps its per-side ordering.
    let mut body = String::from("35=W|34=10|52=20260202-00:00:14.005|55=USDKRW|268=10");
    for i in 0..5 {
        body.push_str(&format!("|269=0|270=14{:02}.00|271=1000000", 50 - i));
        body.push_str(&format!("|269=1|270=14{:02}.00|271=2000000", 53 + i));
    }
    let msg = parse(&body).unwrap();
    assert_eq!(msg.len(), 10);
    assert_eq!(msg.side_depth(MdEntryType::Bid), 5);
    assert_eq!(msg.side_depth(MdEntryType::Offer), 5);
    let bids: Vec<_> = msg
        .entries()
        .iter()
        .filter(|e| e.entry_type == MdEntryType::Bid)
        .map(|e| (e.level, e.price))
        .collect();
    assert_eq!(bids, vec![(0, 145_000), (1, 144_900), (2, 144_800), (3, 144_700), (4, 144_600)]);
    let asks: Vec<_> = msg
        .entries()
        .iter()
        .filter(|e| e.entry_type == MdEntryType::Offer)
        .map(|e| (e.level, e.price))
        .collect();
    assert_eq!(asks, vec![(0, 145_300), (1, 145_400), (2, 145_500), (3, 145_600), (4, 145_700)]);
}

#[test]
fn an_explicit_md_price_level_overrides_the_implied_index() {
    let msg = parse(
        "35=W|34=11|52=20260202-00:00:14.005|55=USDKRW|268=2|\
         269=0|1023=3|270=1448.00|271=1000000|\
         269=0|1023=1|270=1450.00|271=1000000",
    )
    .unwrap();
    assert_eq!(msg.entries()[0].level, 2, "1023 is one-based");
    assert!(msg.entries()[0].has_explicit_level);
    assert_eq!(msg.entries()[1].level, 0);
    assert_eq!(msg.side_depth(MdEntryType::Bid), 3, "depth follows the deepest level");
}

#[test]
fn number_of_orders_is_carried_when_the_venue_sends_it() {
    let msg = parse(
        "35=W|34=12|52=20260202-00:00:14.005|55=USDKRW|268=1|\
         269=0|270=1450.00|271=1000000|346=4",
    )
    .unwrap();
    assert!(msg.entries()[0].has_order_count);
    assert_eq!(msg.entries()[0].order_count, 4);

    let msg = parse(TWO_SIDED_SNAPSHOT).unwrap();
    assert!(!msg.entries()[0].has_order_count, "SMBS sends no 346");
    assert_eq!(msg.entries()[0].order_count, 0);
}

#[test]
fn absent_optional_fields_are_flagged_not_faked() {
    let msg = parse("35=W|34=13|52=20260202-00:00:14.005|55=USDKRW|268=1|269=0|270=1450.00")
        .unwrap();
    let e = msg.entries()[0];
    assert!(e.has_price);
    assert!(!e.has_size);
    assert_eq!(e.size, 0);
    assert!(!e.has_date && !e.has_time);
    assert_eq!(e.venue_time(), None, "a half-present venue time is no venue time");
    assert_eq!(msg.venue_time(), None);
}

#[test]
fn a_declared_entry_count_that_does_not_match_is_rejected() {
    let err = parse(
        "35=W|34=14|52=20260202-00:00:14.005|55=USDKRW|268=3|\
         269=0|270=1450.00|271=1000000",
    )
    .unwrap_err();
    assert_eq!(err, FixError::EntryCount { declared: 3, found: 1 });

    // Zero entries declared and none sent is a legal empty book message.
    let msg = parse("35=W|34=15|52=20260202-00:00:14.005|55=USDKRW|268=0").unwrap();
    assert!(msg.is_empty());
    assert_eq!(msg.len(), 0);
}

#[test]
fn malformed_groups_and_values_are_named() {
    // An entry without 269 cannot be placed.
    let err = parse("35=W|34=16|52=20260202-00:00:14.005|55=USDKRW|268=1|270=1450.00|271=1")
        .unwrap_err();
    assert_eq!(err, FixError::GroupTagOutsideEntry { tag: 270 });

    // An unknown update action.
    let err = parse("35=X|34=17|52=20260202-00:00:14.005|268=1|279=9|269=2|270=1451.00")
        .unwrap_err();
    assert_eq!(err, FixError::UnknownUpdateAction { found: b'9' });

    // A price with more precision than the scale keeps.
    let err = parse("35=W|34=18|52=20260202-00:00:14.005|55=USDKRW|268=1|269=0|270=1450.005")
        .unwrap_err();
    assert_eq!(err, FixError::Precision { tag: 270 });

    // A missing sequence number: the session layer cannot do its job without it.
    let err = parse("35=W|52=20260202-00:00:14.005|55=USDKRW|268=0").unwrap_err();
    assert_eq!(err, FixError::MissingTag { tag: 34 });
}

#[test]
fn a_non_market_data_message_is_refused_by_this_decoder() {
    let err = parse(HEARTBEAT).unwrap_err();
    assert_eq!(err, FixError::UnexpectedMsgType { found: b'0' });
    // An execution report belongs to the order session, not here.
    let err = parse("35=8|34=1|52=20260202-00:00:14.005").unwrap_err();
    assert_eq!(err, FixError::UnexpectedMsgType { found: b'8' });
}

#[test]
fn more_entries_than_the_buffer_holds_is_an_error_not_a_truncation() {
    let count = MD_MAX_ENTRIES + 1;
    let mut body = format!("35=W|34=19|52=20260202-00:00:14.005|55=USDKRW|268={count}");
    for _ in 0..count {
        body.push_str("|269=0|270=1450.00|271=1000000");
    }
    let err = parse(&body).unwrap_err();
    assert_eq!(
        err,
        FixError::TooManyEntries { declared: count as u32, max: MD_MAX_ENTRIES as u32 }
    );
}

#[test]
fn the_scale_pair_is_what_turns_text_into_integers() {
    use jeed_fix::FixScales;

    // Padding zeros carry nothing, so a whole-won price still reads on S0.
    let raw = wrap(TWO_SIDED_SNAPSHOT);
    let f = frame(&raw).unwrap();
    let s0 = parse_md_message(&f, FixScales::new(Scale::S0, Scale::S0)).unwrap();
    assert_eq!(s0.entries()[0].price, 1_450);
    let s4 = parse_md_message(&f, FixScales::new(Scale::S4, Scale::S0)).unwrap();
    assert_eq!(s4.entries()[0].price, 14_500_000);

    // A tick that does not survive the scale does not become a rounded price.
    let raw = wrap("35=W|34=21|52=20260202-00:00:14.005|55=USDKRW|268=1|269=0|270=1450.90|271=1");
    let f = frame(&raw).unwrap();
    assert_eq!(
        parse_md_message(&f, FixScales::new(Scale::S0, Scale::S0)),
        Err(FixError::Precision { tag: 270 }),
        "S0 would silently drop the 0.9 SMBS actually quoted"
    );
    assert_eq!(
        parse_md_message(&f, FixScales::new(Scale::S2, Scale::S0)).unwrap().entries()[0].price,
        145_090
    );
}

#[test]
fn unknown_entry_types_and_statistics_are_carried_not_dropped() {
    let msg = parse(
        "35=W|34=20|52=20260202-00:00:14.005|55=USDKRW|268=2|\
         269=4|270=1450.00|269=Z|270=1451.00",
    )
    .unwrap();
    assert_eq!(msg.entries()[0].entry_type, MdEntryType::OpeningPrice);
    assert_eq!(msg.entries()[1].entry_type, MdEntryType::Other { code: b'Z' });
    assert!(!msg.entries()[0].entry_type.is_book_side());
    assert_eq!(msg.side_depth(MdEntryType::Bid), 0);
}

#[test]
fn poss_dup_is_surfaced_for_the_session_layer() {
    let msg = parse("35=W|34=2|43=Y|52=20260202-00:00:01.005|55=USDKRW|268=0").unwrap();
    assert!(msg.poss_dup);
}
