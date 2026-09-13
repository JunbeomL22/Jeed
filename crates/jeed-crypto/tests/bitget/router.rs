//! Tests for `src/bitget/router.rs` — the `arg` envelope with `instType`,
//! and the bare `ping`.

#[allow(dead_code)]
#[path = "../recv/sink.rs"]
mod sink;

use crate::common::{BOOKS5, BOOKS_SNAPSHOT, BOOKS_UPDATE, RECV_NS, TRADES, linear_btcusdt, spot_btcusdt};
use jeed_crypto::bitget::router::{Router, URL, inst_type};
use jeed_crypto::recv::{Channel, ChannelSet, MAX_KEEPALIVE_LEN, Router as _, RouterError, Subscription};
use jeed_wire::{Venue, WireKind};
use sink::Collect;

fn both() -> ChannelSet {
    ChannelSet::EMPTY.with(Channel::Trade).with(Channel::Book)
}

fn linear() -> Router {
    Router::new(Venue::BitgetLinear, vec![Subscription::new(linear_btcusdt(), both())]).unwrap()
}

#[test]
fn one_url_two_inst_types() {
    assert!(URL.ends_with("/v2/ws/public"));
    assert_eq!(inst_type(Venue::BitgetSpot), Some("SPOT"));
    assert_eq!(inst_type(Venue::BitgetLinear), Some("USDT-FUTURES"));
    assert_eq!(inst_type(Venue::Okx), None);
    assert_eq!(Router::new(Venue::Okx, vec![]).unwrap_err(), RouterError::NotCrypto(Venue::Okx));
}

#[test]
fn subscribes_with_inst_type_and_depth_in_the_channel_name() {
    let subs = linear().subscriptions();
    assert_eq!(
        std::str::from_utf8(&subs[0]).unwrap(),
        r#"{"op":"subscribe","args":[{"instType":"USDT-FUTURES","channel":"trade","instId":"BTCUSDT"},{"instType":"USDT-FUTURES","channel":"books","instId":"BTCUSDT"}]}"#
    );
    let r = Router::new(Venue::BitgetSpot, vec![Subscription::with_depth(spot_btcusdt(), both(), 5)]).unwrap();
    assert_eq!(r.args()[1], ("books5".to_owned(), "BTCUSDT".to_owned()));
}

#[test]
fn trades_are_one_record_per_print() {
    let mut r = linear();
    let mut sink = Collect::new();
    assert_eq!(r.route(TRADES, RECV_NS, &mut sink).unwrap(), 2);
    assert!(sink.records.iter().all(|rec| rec.kind() == Ok(WireKind::Trade)));
    assert_eq!(sink.last().header.venue(), Ok(Venue::BitgetLinear));
}

#[test]
fn books_frames_route_by_action() {
    let mut r = linear();
    let mut sink = Collect::new();
    assert_eq!(r.route(BOOKS_SNAPSHOT, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::Quote));
    assert_eq!(r.route(BOOKS_UPDATE, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::SnapshotDelta));
}

#[test]
fn books5_on_spot_is_a_snapshot() {
    let mut r = Router::new(Venue::BitgetSpot, vec![Subscription::with_depth(spot_btcusdt(), both(), 5)]).unwrap();
    let mut sink = Collect::new();
    assert_eq!(r.route(BOOKS5, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::Quote));
    assert_eq!(sink.last().header.venue(), Ok(Venue::BitgetSpot));
}

#[test]
fn conversation_routes_as_nothing() {
    let mut r = linear();
    let mut sink = Collect::new();
    let ack = br#"{"event":"subscribe","arg":{"instType":"USDT-FUTURES","channel":"trade","instId":"BTCUSDT"}}"#;
    assert_eq!(r.route(ack, RECV_NS, &mut sink).unwrap(), 0);
    assert_eq!(r.route(b"pong", RECV_NS, &mut sink).unwrap(), 0);
    let other = String::from_utf8(TRADES.to_vec()).unwrap().replace(r#""instId":"BTCUSDT""#, r#""instId":"ETHUSDT""#);
    assert_eq!(r.route(other.as_bytes(), RECV_NS, &mut sink).unwrap(), 0);
    assert_eq!(sink.len(), 0);
}

#[test]
fn ping_is_literal_text() {
    let mut r = linear();
    let mut out = [0u8; MAX_KEEPALIVE_LEN];
    let n = r.keepalive(1, &mut out).expect("due at once");
    assert_eq!(&out[..n], b"ping");
    assert_eq!(r.keepalive(2, &mut out), None);
}

#[test]
fn depth_seven_and_delta_are_refused_and_rest_is_not_offered() {
    let e = Router::new(Venue::BitgetSpot, vec![Subscription::with_depth(spot_btcusdt(), both(), 7)]).unwrap_err();
    assert_eq!(e, RouterError::Depth { venue: Venue::BitgetSpot, depth: 7 });
    let e = Router::new(Venue::BitgetSpot, vec![Subscription::new(spot_btcusdt(), both().with(Channel::Delta))]).unwrap_err();
    assert_eq!(e, RouterError::Unsupported { venue: Venue::BitgetSpot, channel: Channel::Delta });
    let mut r = linear();
    assert!(r.rest_books().is_empty());
    assert!(r.route_rest(0, b"{}", RECV_NS, &mut Collect::new()).is_err());
}
