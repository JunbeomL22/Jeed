//! Tests for `src/recv/filter.rs`.

use super::builders::{SCALES, TRADE_INCREMENTAL, TWO_SIDED_SNAPSHOT, wrap};
use jeed_fix::recv::{SymbolFilter, parse_symbol_list};
use jeed_fix::{MdMessage, frame, parse_md_message};

fn parse(body: &str) -> MdMessage {
    let raw = wrap(body);
    let f = frame(&raw).expect("framing");
    parse_md_message(&f, SCALES).expect("decoding")
}

#[test]
fn an_empty_set_keeps_everything() {
    // The opposite of `jeed_krx`'s trcode set, and on purpose: a FIX session
    // carries what we subscribed to, so this is a backstop rather than the
    // primary filter.
    let f = SymbolFilter::all();
    assert!(f.is_empty());
    assert!(f.allows(b"USDKRW"));
    assert!(f.allows(b""));
    assert!(f.allows_message(&parse(TWO_SIDED_SNAPSHOT)));
}

#[test]
fn a_listed_set_keeps_exactly_what_is_listed() {
    let f = SymbolFilter::new([b"USDKRW".as_slice(), b"EURKRW".as_slice()]);
    assert_eq!(f.len(), 2);
    assert!(f.allows(b"USDKRW"));
    assert!(f.allows(b"EURKRW"));
    assert!(!f.allows(b"JPYKRW"));
    assert!(!f.allows(b"USDKR"), "a prefix is a different symbol");
    assert!(!f.allows(b"USDKRWX"));
    assert!(!f.allows(b""));

    // Sorted and deduplicated, so lookup is a binary search.
    let f = SymbolFilter::new([b"B".as_slice(), b"A".as_slice(), b"B".as_slice()]);
    assert_eq!(f.len(), 2);
    assert_eq!(f.symbols().collect::<Vec<_>>(), vec![b"A".as_slice(), b"B".as_slice()]);
}

#[test]
fn a_symbol_past_the_decoder_s_capacity_is_compared_on_the_same_prefix() {
    // The decoder truncates `55` to FIX_TEXT_LEN, so the filter must, or the
    // two would disagree about what they are comparing.
    let long = [b'X'; 40];
    let f = SymbolFilter::new([long.as_slice()]);
    assert!(f.allows(&long[..32]));
    assert!(!f.allows(&long[..31]));
}

#[test]
fn a_message_passes_if_any_of_its_symbols_does() {
    // 35=W names its instrument once in the header.
    let keep = SymbolFilter::new([b"USDKRW".as_slice()]);
    let drop = SymbolFilter::new([b"EURKRW".as_slice()]);
    assert!(keep.allows_message(&parse(TWO_SIDED_SNAPSHOT)));
    assert!(!drop.allows_message(&parse(TWO_SIDED_SNAPSHOT)));

    // 35=X carries it inside the group, possibly once per instrument.
    assert!(keep.allows_message(&parse(TRADE_INCREMENTAL)));
    assert!(!drop.allows_message(&parse(TRADE_INCREMENTAL)));

    let mixed = parse(
        "35=X|34=9|52=20260202-00:00:14.005|262=R|268=2|\
         279=0|269=2|55=USDKRW|270=1451.00|271=1|272=20260202|273=00:00:14.000|\
         279=0|269=2|55=JPYKRW|270=1451.10|271=2|272=20260202|273=00:00:15.000",
    );
    assert!(keep.allows_message(&mixed), "one wanted instrument keeps the message");
    assert!(
        !SymbolFilter::new([b"GBPKRW".as_slice()]).allows_message(&mixed),
        "and none keeps nothing"
    );
}

#[test]
fn a_message_that_names_no_symbol_is_left_to_the_adapter() {
    // The venue is identifying the instrument by MDReqID alone; refusing it
    // here would be this stage deciding something only the adapter can know.
    let msg = parse("35=W|34=1|52=20260202-00:00:14.005|262=USDKRW-SMBS|268=1|269=0|270=1450.00");
    assert!(msg.symbol.is_empty());
    assert!(SymbolFilter::new([b"EURKRW".as_slice()]).allows_message(&msg));
}

#[test]
fn a_list_file_ignores_comments_and_blanks() {
    let f = parse_symbol_list(
        "# the pairs this session wants\n\
         USDKRW\n\
         \n\
           EURKRW   # with a trailing note\n\
         \n",
    );
    assert_eq!(f.len(), 2);
    assert!(f.allows(b"USDKRW"));
    assert!(f.allows(b"EURKRW"));
    assert!(parse_symbol_list("# nothing but comments\n").is_empty());
}
