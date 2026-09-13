//! Tests for `src/recv/route.rs` — the conf vocabulary and the request
//! types every router speaks in.

use jeed_crypto::recv::{Channel, ChannelSet, HttpRequest, Method, RouterError, Subscription, route::lookup};
use jeed_crypto::Instrument;
use jeed_wire::Venue;

#[test]
fn channels_round_trip_through_their_spelling() {
    for c in Channel::ALL {
        assert_eq!(Channel::parse(c.as_str()), Some(c));
        assert_eq!(c.to_string(), c.as_str());
    }
    assert_eq!(Channel::parse("orderbook"), None);
    assert_eq!(Channel::parse("Trade"), None, "case matters");
}

#[test]
fn a_set_holds_what_it_was_given_in_a_fixed_order() {
    let set: ChannelSet = [Channel::Delta, Channel::Trade].into_iter().collect();
    assert!(set.contains(Channel::Trade));
    assert!(set.contains(Channel::Delta));
    assert!(!set.contains(Channel::Bbo));
    assert_eq!(set.iter().collect::<Vec<_>>(), [Channel::Trade, Channel::Delta]);
    assert_eq!(set.to_string(), "trade,delta");
    assert!(ChannelSet::EMPTY.is_empty());
    assert_eq!(ChannelSet::EMPTY.to_string(), "");
}

#[test]
fn lookup_matches_the_venues_spelling_exactly() {
    let subs = vec![
        Subscription::new(Instrument::new(Venue::BinanceSpot, b"BTCUSDT", 2, 5).unwrap(), ChannelSet::EMPTY.with(Channel::Trade)),
        Subscription::with_depth(Instrument::new(Venue::BinanceSpot, b"ETHUSDT", 2, 4).unwrap(), ChannelSet::EMPTY.with(Channel::Book), 20),
    ];
    assert_eq!(lookup(&subs, b"ETHUSDT").map(|(i, _)| i), Some(1));
    assert_eq!(lookup(&subs, b"ethusdt").map(|(i, _)| i), None);
    assert_eq!(lookup(&subs, b"BTCUSD").map(|(i, _)| i), None);
    assert_eq!(subs[1].depth, Some(20));
}

#[test]
fn requests_display_as_a_request_line() {
    assert_eq!(HttpRequest::get("https://a/b?c=1").to_string(), "GET https://a/b?c=1");
    assert_eq!(HttpRequest::post("https://a/t").method, Method::Post);
    assert_eq!(Method::Post.as_str(), "POST");
}

#[test]
fn errors_say_what_and_where() {
    assert_eq!(RouterError::NotCrypto(Venue::Krx).to_string(), "KRX is not a crypto venue");
    assert_eq!(
        RouterError::Unsupported { venue: Venue::GateSpot, channel: Channel::Book }.to_string(),
        "GATE_SPOT has no `book` channel"
    );
    assert_eq!(RouterError::Depth { venue: Venue::BybitSpot, depth: 7 }.to_string(), "BYBIT_SPOT has no book of depth 7");
    assert_eq!(RouterError::Duplicate { symbol: "BTCUSDT".into() }.to_string(), "BTCUSDT is listed twice");
    assert_eq!(
        RouterError::WrongVenue { expected: Venue::Upbit, found: Venue::Bithumb }.to_string(),
        "instrument is on BITHUMB but the feed is UPBIT"
    );
    assert_eq!(RouterError::Ticket("no `token`").to_string(), "ticket: no `token`");
}
