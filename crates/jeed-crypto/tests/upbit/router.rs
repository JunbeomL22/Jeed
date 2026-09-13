//! Tests for `src/upbit/router.rs` — `type` + `code`, both spellings, and
//! the ticket subscription. Bithumb rides the same router.

#[allow(dead_code)]
#[path = "../recv/sink.rs"]
mod sink;

use crate::common::{ORDERBOOK, ORDERBOOK_SIMPLE, RECV_NS, TRADE, TRADE_SIMPLE, krw_btc};
use jeed_crypto::Instrument;
use jeed_crypto::recv::{Channel, ChannelSet, Router as _, RouterError, Subscription};
use jeed_crypto::upbit::router::{BITHUMB_URL, Router, UPBIT_URL, url};
use jeed_wire::{Venue, WireKind};
use sink::Collect;

fn both() -> ChannelSet {
    ChannelSet::EMPTY.with(Channel::Trade).with(Channel::Book)
}

fn upbit() -> Router {
    Router::new(Venue::Upbit, vec![Subscription::new(krw_btc(), both())]).unwrap()
}

#[test]
fn each_venue_has_its_url_and_nobody_else_does() {
    assert_eq!(url(Venue::Upbit), Some(UPBIT_URL));
    assert_eq!(url(Venue::Bithumb), Some(BITHUMB_URL));
    assert_eq!(url(Venue::Okx), None);
}

#[test]
fn the_subscription_is_one_array_with_a_ticket() {
    let eth = Instrument::new(Venue::Upbit, b"KRW-ETH", 0, 8).unwrap();
    let r = Router::new(
        Venue::Upbit,
        vec![
            Subscription::new(krw_btc(), both()),
            Subscription::new(eth, ChannelSet::EMPTY.with(Channel::Trade)),
        ],
    )
    .unwrap();
    let subs = r.subscriptions();
    assert_eq!(subs.len(), 1);
    assert_eq!(
        std::str::from_utf8(&subs[0]).unwrap(),
        r#"[{"ticket":"jeed"},{"type":"trade","codes":["KRW-BTC","KRW-ETH"]},{"type":"orderbook","codes":["KRW-BTC"]}]"#
    );
}

#[test]
fn default_spelling_routes() {
    let mut r = upbit();
    let mut sink = Collect::new();
    assert_eq!(r.route(TRADE, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::Trade));
    assert_eq!(r.route(ORDERBOOK, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::Quote));
    assert_eq!(sink.last().header.venue(), Ok(Venue::Upbit));
}

#[test]
fn simple_spelling_routes_too() {
    let mut r = upbit();
    let mut sink = Collect::new();
    assert_eq!(r.route(TRADE_SIMPLE, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::Trade));
    assert_eq!(r.route(ORDERBOOK_SIMPLE, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::Quote));
}

#[test]
fn a_status_reply_and_an_unknown_market_route_as_nothing() {
    let mut r = upbit();
    let mut sink = Collect::new();
    assert_eq!(r.route(br#"{"status":"UP"}"#, RECV_NS, &mut sink).unwrap(), 0);
    let eth = TRADE.to_vec().into_iter().collect::<Vec<u8>>();
    let eth = String::from_utf8(eth).unwrap().replace("KRW-BTC", "KRW-ETH");
    assert_eq!(r.route(eth.as_bytes(), RECV_NS, &mut sink).unwrap(), 0);
    assert_eq!(sink.len(), 0);
}

#[test]
fn a_bithumb_instrument_makes_bithumb_records() {
    let inst = Instrument::new(Venue::Bithumb, b"KRW-BTC", 0, 8).unwrap();
    let mut r = Router::new(Venue::Bithumb, vec![Subscription::new(inst, both())]).unwrap();
    let mut sink = Collect::new();
    assert_eq!(r.venue(), Venue::Bithumb);
    assert_eq!(r.route(TRADE, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().header.venue(), Ok(Venue::Bithumb));
}

#[test]
fn an_upbit_instrument_on_a_bithumb_router_is_refused() {
    let e = Router::new(Venue::Bithumb, vec![Subscription::new(krw_btc(), both())]).unwrap_err();
    assert_eq!(e, RouterError::WrongVenue { expected: Venue::Bithumb, found: Venue::Upbit });
}

#[test]
fn delta_bbo_and_depth_are_refused() {
    let e = Router::new(Venue::Upbit, vec![Subscription::new(krw_btc(), both().with(Channel::Delta))]).unwrap_err();
    assert_eq!(e, RouterError::Unsupported { venue: Venue::Upbit, channel: Channel::Delta });
    let e = Router::new(Venue::Upbit, vec![Subscription::with_depth(krw_btc(), both(), 5)]).unwrap_err();
    assert_eq!(e, RouterError::Depth { venue: Venue::Upbit, depth: 5 });
}

#[test]
fn another_venue_is_not_this_protocol() {
    assert_eq!(Router::new(Venue::Okx, vec![]).unwrap_err(), RouterError::NotCrypto(Venue::Okx));
}
