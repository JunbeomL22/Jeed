//! Tests for `src/bybit/router.rs` — `topic`, `subscribe` in tens, and
//! `{"op":"ping"}`.

#[allow(dead_code)]
#[path = "../recv/sink.rs"]
mod sink;

use crate::common::{ORDERBOOK_DELTA, ORDERBOOK_SNAPSHOT, PUBLIC_TRADE, RECV_NS, linear_btcusdt, spot_btcusdt};
use jeed_crypto::Instrument;
use jeed_crypto::bybit::router::{LINEAR_URL, PING, Router, SPOT_URL, url};
use jeed_crypto::recv::{Channel, ChannelSet, MAX_KEEPALIVE_LEN, Router as _, RouterError, Subscription};
use jeed_wire::{Venue, WireKind};
use sink::Collect;

fn both() -> ChannelSet {
    ChannelSet::EMPTY.with(Channel::Trade).with(Channel::Book)
}

fn linear() -> Router {
    Router::new(Venue::BybitLinear, vec![Subscription::new(linear_btcusdt(), both())]).unwrap()
}

#[test]
fn each_venue_has_its_url() {
    assert_eq!(url(Venue::BybitSpot), Some(SPOT_URL));
    assert_eq!(url(Venue::BybitLinear), Some(LINEAR_URL));
    assert_eq!(url(Venue::Upbit), None);
    assert_eq!(Router::new(Venue::Upbit, vec![]).unwrap_err(), RouterError::NotCrypto(Venue::Upbit));
}

#[test]
fn topics_take_the_default_depth() {
    assert_eq!(linear().topics(), ["publicTrade.BTCUSDT", "orderbook.50.BTCUSDT"]);
    let r = Router::new(Venue::BybitLinear, vec![Subscription::with_depth(linear_btcusdt(), both(), 1)]).unwrap();
    assert_eq!(r.topics(), ["publicTrade.BTCUSDT", "orderbook.1.BTCUSDT"]);
}

#[test]
fn subscribes_ten_topics_per_message() {
    let subs: Vec<Subscription> = (0..6)
        .map(|i| {
            let symbol = format!("SYM{i}USDT");
            Subscription::new(Instrument::new(Venue::BybitSpot, symbol.as_bytes(), 2, 3).unwrap(), both())
        })
        .collect();
    let r = Router::new(Venue::BybitSpot, subs).unwrap();
    let msgs = r.subscriptions();
    assert_eq!(msgs.len(), 2, "twelve topics, ten per message");
    let first = std::str::from_utf8(&msgs[0]).unwrap();
    assert!(first.starts_with(r#"{"op":"subscribe","args":["publicTrade.SYM0USDT","orderbook.50.SYM0USDT","#), "{first}");
    assert_eq!(first.matches("publicTrade").count() + first.matches("orderbook").count(), 10);
}

#[test]
fn trades_are_one_record_per_print() {
    let mut r = linear();
    let mut sink = Collect::new();
    assert_eq!(r.route(PUBLIC_TRADE, RECV_NS, &mut sink).unwrap(), 2);
    assert!(sink.records.iter().all(|rec| rec.kind() == Ok(WireKind::Trade)));
    assert_eq!(sink.last().header.venue(), Ok(Venue::BybitLinear));
}

#[test]
fn orderbook_frames_route_by_type() {
    let mut r = linear();
    let mut sink = Collect::new();
    assert_eq!(r.route(ORDERBOOK_SNAPSHOT, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::Quote));
    assert_eq!(r.route(ORDERBOOK_DELTA, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::SnapshotDelta));
}

#[test]
fn the_spot_venue_byte_comes_from_the_instrument() {
    let mut r = Router::new(Venue::BybitSpot, vec![Subscription::new(spot_btcusdt(), both())]).unwrap();
    let mut sink = Collect::new();
    assert_eq!(r.route(PUBLIC_TRADE, RECV_NS, &mut sink).unwrap(), 2);
    assert_eq!(sink.last().header.venue(), Ok(Venue::BybitSpot));
}

#[test]
fn conversation_routes_as_nothing() {
    let mut r = linear();
    let mut sink = Collect::new();
    let ack = br#"{"success":true,"ret_msg":"subscribe","conn_id":"a","req_id":"","op":"subscribe"}"#;
    assert_eq!(r.route(ack, RECV_NS, &mut sink).unwrap(), 0);
    let pong = br#"{"success":true,"ret_msg":"pong","conn_id":"a","op":"ping"}"#;
    assert_eq!(r.route(pong, RECV_NS, &mut sink).unwrap(), 0);
    let other = String::from_utf8(PUBLIC_TRADE.to_vec()).unwrap().replace("publicTrade.BTCUSDT", "publicTrade.ETHUSDT");
    assert_eq!(r.route(other.as_bytes(), RECV_NS, &mut sink).unwrap(), 0);
    assert_eq!(sink.len(), 0);
}

#[test]
fn ping_is_an_op_on_a_schedule() {
    let mut r = linear();
    let mut out = [0u8; MAX_KEEPALIVE_LEN];
    let n = r.keepalive(5, &mut out).expect("due at once");
    assert_eq!(&out[..n], PING);
    assert_eq!(r.keepalive(6, &mut out), None);
}

#[test]
fn depths_are_the_venues_own() {
    let ok = Router::new(Venue::BybitLinear, vec![Subscription::with_depth(linear_btcusdt(), both(), 500)]);
    assert!(ok.is_ok());
    let e = Router::new(Venue::BybitSpot, vec![Subscription::with_depth(spot_btcusdt(), both(), 500)]).unwrap_err();
    assert_eq!(e, RouterError::Depth { venue: Venue::BybitSpot, depth: 500 });
    let e = Router::new(Venue::BybitSpot, vec![Subscription::new(spot_btcusdt(), both().with(Channel::Delta))]).unwrap_err();
    assert_eq!(e, RouterError::Unsupported { venue: Venue::BybitSpot, channel: Channel::Delta });
}
