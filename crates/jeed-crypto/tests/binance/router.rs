//! Tests for `src/binance/router.rs` — the combined-stream envelope and
//! `SUBSCRIBE`.

#[allow(dead_code)]
#[path = "../recv/sink.rs"]
mod sink;

use crate::common::{
    FUT_AGG_TRADE, FUT_DELTA, RECV_NS, SPOT_BOOK_TICKER, SPOT_DELTA, SPOT_SNAPSHOT, SPOT_TRADE,
    futures_btcusdt, spot_btcusdt,
};
use jeed_crypto::binance::router::{FUTURES_URL, Market, Router, SPOT_URL};
use jeed_crypto::recv::{Channel, ChannelSet, Router as _, RouterError, Subscription};
use jeed_crypto::{CryptoError, Instrument};
use jeed_wire::{Venue, WireKind};
use sink::Collect;

fn all() -> ChannelSet {
    Channel::ALL.into_iter().collect()
}

fn spot() -> Router {
    Router::new(Market::Spot, vec![Subscription::new(spot_btcusdt(), all())]).unwrap()
}

fn futures() -> Router {
    Router::new(Market::Futures, vec![Subscription::new(futures_btcusdt(), all())]).unwrap()
}

/// Wraps a flat frame the way `/stream` does.
fn combined(stream: &str, data: &[u8]) -> Vec<u8> {
    let mut v = format!(r#"{{"stream":"{stream}","data":"#).into_bytes();
    v.extend_from_slice(data);
    v.push(b'}');
    v
}

#[test]
fn the_urls_are_the_combined_stream_ones() {
    assert_eq!(Market::Spot.url(), SPOT_URL);
    assert_eq!(Market::Futures.url(), FUTURES_URL);
    assert!(SPOT_URL.ends_with("/stream"));
}

#[test]
fn spot_subscribes_to_every_stream_in_one_message() {
    let subs = spot().subscriptions();
    assert_eq!(subs.len(), 1);
    assert_eq!(
        std::str::from_utf8(&subs[0]).unwrap(),
        r#"{"method":"SUBSCRIBE","params":["btcusdt@trade","btcusdt@bookTicker","btcusdt@depth10@100ms","btcusdt@depth@100ms"],"id":1}"#
    );
}

#[test]
fn futures_trades_are_agg_trades_and_depth_is_configurable() {
    let sub = Subscription::with_depth(futures_btcusdt(), all(), 20);
    let r = Router::new(Market::Futures, vec![sub]).unwrap();
    assert_eq!(
        r.streams(),
        ["btcusdt@aggTrade", "btcusdt@bookTicker", "btcusdt@depth20@100ms", "btcusdt@depth@100ms"]
    );
}

#[test]
fn each_spot_stream_reaches_its_decoder() {
    let mut r = spot();
    let mut sink = Collect::new();

    assert_eq!(r.route(&combined("btcusdt@trade", SPOT_TRADE), RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::Trade));
    assert_eq!(sink.last().header.venue(), Ok(Venue::BinanceSpot));
    assert_eq!(sink.last().header.symbol_bytes(), b"BTCUSDT");

    assert_eq!(r.route(&combined("btcusdt@bookTicker", SPOT_BOOK_TICKER), RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::Quote));
    assert_eq!(sink.last().header.depth, 1);

    assert_eq!(r.route(&combined("btcusdt@depth10@100ms", SPOT_SNAPSHOT), RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::Quote));
    assert_eq!(sink.last().header.depth, 3);

    assert_eq!(r.route(&combined("btcusdt@depth@100ms", SPOT_DELTA), RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::SnapshotDelta));
    assert_eq!(sink.len(), 4);
}

#[test]
fn futures_streams_reach_the_futures_decoders() {
    let mut r = futures();
    let mut sink = Collect::new();

    assert_eq!(r.route(&combined("btcusdt@aggTrade", FUT_AGG_TRADE), RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::Trade));
    assert_eq!(sink.last().header.venue(), Ok(Venue::BinanceFutures));

    assert_eq!(r.route(&combined("btcusdt@depth@100ms", FUT_DELTA), RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::SnapshotDelta));
}

#[test]
fn an_acknowledgement_routes_as_nothing() {
    let mut sink = Collect::new();
    assert_eq!(spot().route(br#"{"result":null,"id":1}"#, RECV_NS, &mut sink).unwrap(), 0);
    assert_eq!(sink.len(), 0);
}

#[test]
fn a_stream_for_another_symbol_routes_as_nothing() {
    let mut sink = Collect::new();
    let frame = combined("ethusdt@trade", SPOT_TRADE);
    assert_eq!(spot().route(&frame, RECV_NS, &mut sink).unwrap(), 0);
}

#[test]
fn a_frame_the_decoder_refuses_is_an_error_and_publishes_nothing() {
    let mut sink = Collect::new();
    let frame = combined("btcusdt@trade", br#"{"e":"trade","s":"ETHUSDT","p":"1","q":"1","m":true}"#);
    assert_eq!(spot().route(&frame, RECV_NS, &mut sink), Err(CryptoError::SymbolMismatch));
    assert_eq!(sink.len(), 0);
}

#[test]
fn the_envelope_is_read_in_either_order() {
    let mut sink = Collect::new();
    let mut frame = br#"{"data":"#.to_vec();
    frame.extend_from_slice(SPOT_TRADE);
    frame.extend_from_slice(br#","stream":"btcusdt@trade"}"#);
    assert_eq!(spot().route(&frame, RECV_NS, &mut sink).unwrap(), 1);
}

#[test]
fn a_depth_binance_does_not_offer_is_refused() {
    let sub = Subscription::with_depth(spot_btcusdt(), all(), 7);
    let e = Router::new(Market::Spot, vec![sub]).unwrap_err();
    assert_eq!(e, RouterError::Depth { venue: Venue::BinanceSpot, depth: 7 });
}

#[test]
fn an_instrument_on_the_other_market_is_refused() {
    let e = Router::new(Market::Spot, vec![Subscription::new(futures_btcusdt(), all())]).unwrap_err();
    assert_eq!(e, RouterError::WrongVenue { expected: Venue::BinanceSpot, found: Venue::BinanceFutures });
}

#[test]
fn no_instruments_and_no_channels_are_refused() {
    assert_eq!(Router::new(Market::Spot, vec![]).unwrap_err(), RouterError::NoInstruments);
    let e = Router::new(Market::Spot, vec![Subscription::new(spot_btcusdt(), ChannelSet::EMPTY)]).unwrap_err();
    assert_eq!(e, RouterError::NoChannels { symbol: "BTCUSDT".into() });
}

#[test]
fn a_symbol_twice_is_refused() {
    let a = Subscription::new(spot_btcusdt(), all());
    let b = Subscription::new(Instrument::new(Venue::BinanceSpot, b"BTCUSDT", 1, 1).unwrap(), all());
    let e = Router::new(Market::Spot, vec![a, b]).unwrap_err();
    assert_eq!(e, RouterError::Duplicate { symbol: "BTCUSDT".into() });
}

#[test]
fn binance_needs_no_keepalive_no_ticket_and_no_rest() {
    let mut r = spot();
    let mut out = [0u8; jeed_crypto::recv::MAX_KEEPALIVE_LEN];
    assert_eq!(r.keepalive(u64::MAX, &mut out), None);
    assert_eq!(r.ticket(), None);
    assert!(r.rest_books().is_empty());
}
