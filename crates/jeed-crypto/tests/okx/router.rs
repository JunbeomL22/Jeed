//! Tests for `src/okx/router.rs` — the `arg` envelope, `subscribe`, and
//! the bare `ping`.

#[allow(dead_code)]
#[path = "../recv/sink.rs"]
mod sink;

use crate::common::{BOOKS5, BOOKS_SNAPSHOT, BOOKS_UPDATE, RECV_NS, TRADES, btc_usdt};
use jeed_crypto::okx::router::{PING_INTERVAL_NS, Router, URL};
use jeed_crypto::recv::{Channel, ChannelSet, MAX_KEEPALIVE_LEN, Router as _, RouterError, Subscription};
use jeed_wire::{Venue, WireKind};
use sink::Collect;

fn all_three() -> ChannelSet {
    ChannelSet::EMPTY.with(Channel::Trade).with(Channel::Book).with(Channel::Delta)
}

fn okx() -> Router {
    Router::new(vec![Subscription::new(btc_usdt(), all_three())]).unwrap()
}

#[test]
fn the_url_is_the_public_v5_one() {
    assert!(URL.ends_with("/ws/v5/public"));
}

#[test]
fn subscribes_with_one_arg_per_channel() {
    let subs = okx().subscriptions();
    assert_eq!(subs.len(), 1);
    assert_eq!(
        std::str::from_utf8(&subs[0]).unwrap(),
        r#"{"op":"subscribe","args":[{"channel":"trades","instId":"BTC-USDT"},{"channel":"books5","instId":"BTC-USDT"},{"channel":"books","instId":"BTC-USDT"}]}"#
    );
}

#[test]
fn a_trades_frame_is_one_record_per_print() {
    let mut r = okx();
    let mut sink = Collect::new();
    assert_eq!(r.route(TRADES, RECV_NS, &mut sink).unwrap(), 2);
    assert_eq!(sink.len(), 2);
    assert!(sink.records.iter().all(|rec| rec.kind() == Ok(WireKind::Trade)));
    assert_eq!(sink.last().header.venue(), Ok(Venue::Okx));
}

#[test]
fn books_frames_route_by_action_and_books5_is_a_snapshot() {
    let mut r = okx();
    let mut sink = Collect::new();
    assert_eq!(r.route(BOOKS_SNAPSHOT, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::Quote));
    assert_eq!(r.route(BOOKS_UPDATE, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::SnapshotDelta));
    assert_eq!(r.route(BOOKS5, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::Quote));
}

#[test]
fn conversation_routes_as_nothing() {
    let mut r = okx();
    let mut sink = Collect::new();
    let ack = br#"{"event":"subscribe","arg":{"channel":"trades","instId":"BTC-USDT"},"connId":"a4d3ae55"}"#;
    assert_eq!(r.route(ack, RECV_NS, &mut sink).unwrap(), 0);
    assert_eq!(r.route(b"pong", RECV_NS, &mut sink).unwrap(), 0);
    let err = br#"{"event":"error","code":"60012","msg":"Invalid request","connId":"a4d3ae55"}"#;
    assert_eq!(r.route(err, RECV_NS, &mut sink).unwrap(), 0);
    assert_eq!(sink.len(), 0);
}

#[test]
fn a_frame_for_another_instrument_routes_as_nothing() {
    let mut r = okx();
    let mut sink = Collect::new();
    let eth = String::from_utf8(TRADES.to_vec()).unwrap().replace("BTC-USDT", "ETH-USDT");
    assert_eq!(r.route(eth.as_bytes(), RECV_NS, &mut sink).unwrap(), 0);
}

#[test]
fn a_bad_print_ends_the_batch_with_its_error() {
    let mut r = okx();
    let mut sink = Collect::new();
    let bad = String::from_utf8(TRADES.to_vec()).unwrap().replace(r#""px":"64000.4""#, r#""px":"abc""#);
    assert!(r.route(bad.as_bytes(), RECV_NS, &mut sink).is_err());
    assert_eq!(sink.len(), 1, "the print before the bad one was published");
}

#[test]
fn ping_is_literal_text_on_a_schedule() {
    let mut r = okx();
    let mut out = [0u8; MAX_KEEPALIVE_LEN];
    let n = r.keepalive(1_000, &mut out).expect("due at once");
    assert_eq!(&out[..n], b"ping");
    assert_eq!(r.keepalive(1_000 + PING_INTERVAL_NS - 1, &mut out), None);
    assert!(r.keepalive(1_000 + PING_INTERVAL_NS, &mut out).is_some());
}

#[test]
fn bbo_and_depth_are_refused() {
    let e = Router::new(vec![Subscription::new(btc_usdt(), ChannelSet::EMPTY.with(Channel::Bbo))]).unwrap_err();
    assert_eq!(e, RouterError::Unsupported { venue: Venue::Okx, channel: Channel::Bbo });
    let e = Router::new(vec![Subscription::with_depth(btc_usdt(), all_three(), 50)]).unwrap_err();
    assert_eq!(e, RouterError::Depth { venue: Venue::Okx, depth: 50 });
}
