//! Tests for `src/recv/venue.rs` — the switchboard builds every venue's
//! router and refuses the ones that are not crypto.

use crate::sink::Collect;
use jeed_crypto::Instrument;
use jeed_crypto::recv::{Channel, ChannelSet, Router, RouterError, Subscription, VenueRouter};
use jeed_wire::{Venue, WireKind};

/// A subscription that every crypto venue accepts: `trade` only.
fn trade_only(venue: Venue, symbol: &[u8]) -> Vec<Subscription> {
    let inst = Instrument::new(venue, symbol, 2, 4).unwrap();
    vec![Subscription::new(inst, ChannelSet::EMPTY.with(Channel::Trade))]
}

const CRYPTO: [(Venue, &[u8]); 12] = [
    (Venue::BinanceSpot, b"BTCUSDT"),
    (Venue::BinanceFutures, b"BTCUSDT"),
    (Venue::Upbit, b"KRW-BTC"),
    (Venue::Bithumb, b"KRW-BTC"),
    (Venue::Okx, b"BTC-USDT"),
    (Venue::BybitSpot, b"BTCUSDT"),
    (Venue::BybitLinear, b"BTCUSDT"),
    (Venue::BitgetSpot, b"BTCUSDT"),
    (Venue::BitgetLinear, b"BTCUSDT"),
    (Venue::GateSpot, b"BTC_USDT"),
    (Venue::KucoinSpot, b"BTC-USDT"),
    (Venue::KucoinFutures, b"XBTUSDTM"),
];

#[test]
fn every_crypto_venue_builds_and_stamps_its_own_byte() {
    for (venue, symbol) in CRYPTO {
        let r = VenueRouter::new(venue, trade_only(venue, symbol)).unwrap_or_else(|e| panic!("{}: {e}", venue.as_str()));
        assert_eq!(r.venue(), venue);
        assert_eq!(r.subscriptions_list().len(), 1);
        assert!(VenueRouter::is_crypto(venue));
        assert!(!r.subscriptions().is_empty(), "{}: every venue subscribes by message", venue.as_str());
    }
}

#[test]
fn the_non_crypto_venues_are_refused() {
    for venue in [Venue::Krx, Venue::Nxt, Venue::Smbs] {
        assert!(!VenueRouter::is_crypto(venue));
        assert_eq!(VenueRouter::new(venue, vec![]).unwrap_err(), RouterError::NotCrypto(venue));
        assert_eq!(VenueRouter::default_url(venue), None);
    }
}

#[test]
fn every_venue_but_kucoin_has_a_default_url() {
    for (venue, _) in CRYPTO {
        let url = VenueRouter::default_url(venue);
        match venue {
            Venue::KucoinSpot | Venue::KucoinFutures => assert_eq!(url, None, "the address comes from a ticket"),
            _ => assert!(url.is_some_and(|u| u.starts_with("wss://")), "{}: {url:?}", venue.as_str()),
        }
    }
    assert_eq!(VenueRouter::default_url(Venue::Bithumb), Some("wss://ws-api.bithumb.com/websocket/v1"));
    assert_eq!(VenueRouter::default_url(Venue::BybitLinear), Some("wss://stream.bybit.com/v5/public/linear"));
}

#[test]
fn only_gate_and_kucoin_need_rest() {
    for (venue, symbol) in CRYPTO {
        let r = VenueRouter::new(venue, trade_only(venue, symbol)).unwrap();
        let ticket = r.ticket().is_some();
        assert_eq!(ticket, matches!(venue, Venue::KucoinSpot | Venue::KucoinFutures), "{}", venue.as_str());
    }
    let mut delta = trade_only(Venue::GateSpot, b"BTC_USDT");
    delta[0].channels = delta[0].channels.with(Channel::Delta);
    let r = VenueRouter::new(Venue::GateSpot, delta).unwrap().with_rest("http://127.0.0.1:1");
    assert_eq!(r.rest_books().len(), 1);
    assert!(r.rest_books()[0].request.url.starts_with("http://127.0.0.1:1/"));
    // A venue without REST is unchanged by the override.
    let r = VenueRouter::new(Venue::Okx, trade_only(Venue::Okx, b"BTC-USDT")).unwrap().with_rest("http://x");
    assert!(r.rest_books().is_empty());
}

#[test]
fn routing_reaches_the_venue_underneath() {
    let mut r = VenueRouter::new(Venue::BinanceSpot, trade_only(Venue::BinanceSpot, b"BTCUSDT")).unwrap();
    let mut sink = Collect::new();
    let mut frame = br#"{"stream":"btcusdt@trade","data":"#.to_vec();
    frame.extend_from_slice(crate::frames::SPOT_TRADE);
    frame.push(b'}');
    assert_eq!(r.route(&frame, crate::frames::RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::Trade));
    assert!(r.route_rest(0, b"{}", 0, &mut sink).is_err(), "binance asks for no REST body");
    assert!(r.endpoint_from_ticket(b"{}").is_err());
}
