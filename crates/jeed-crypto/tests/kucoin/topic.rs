//! `topic_symbol` — the identity a futures book frame has and nothing else.

use jeed_crypto::kucoin::topic_symbol;

#[test]
fn the_symbol_follows_the_colon() {
    assert_eq!(topic_symbol(b"/market/level2:BTC-USDT"), Some(&b"BTC-USDT"[..]));
    assert_eq!(topic_symbol(b"/market/match:BTC-USDT"), Some(&b"BTC-USDT"[..]));
    assert_eq!(topic_symbol(b"/contractMarket/level2:XBTUSDTM"), Some(&b"XBTUSDTM"[..]));
    assert_eq!(topic_symbol(b"/contractMarket/execution:XBTUSDTM"), Some(&b"XBTUSDTM"[..]));
}

#[test]
fn a_topic_that_names_nothing_is_not_a_symbol() {
    assert_eq!(topic_symbol(b"/market/level2"), None);
    assert_eq!(topic_symbol(b"/market/level2:"), None, "a colon with nothing after it");
    assert_eq!(topic_symbol(b""), None);
}

#[test]
fn a_hyphenated_symbol_survives_whole() {
    // The cut is at the first colon, not at the first separator of any kind:
    // spot symbols have hyphens in them.
    assert_eq!(topic_symbol(b"/market/level2:KCS-BTC"), Some(&b"KCS-BTC"[..]));
}
