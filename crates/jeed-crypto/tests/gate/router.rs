//! Tests for `src/gate/router.rs` — `channel` gated by `event`, timed
//! subscriptions, and the REST start book.

#[allow(dead_code)]
#[path = "../recv/sink.rs"]
mod sink;

use crate::common::{ORDER_BOOK_UPDATE, RECV_NS, REST_ORDER_BOOK, SUBSCRIBE_ACK, TRADE, btc_usdt};
use jeed_crypto::gate::router::{REST_URL, Router, URL};
use jeed_crypto::recv::{Channel, ChannelSet, MAX_KEEPALIVE_LEN, Method, Router as _, RouterError, Subscription};
use jeed_wire::{Venue, WireKind, quote_ext};
use sink::Collect;

fn both() -> ChannelSet {
    ChannelSet::EMPTY.with(Channel::Trade).with(Channel::Delta)
}

fn gate() -> Router {
    Router::new(vec![Subscription::new(btc_usdt(), both())]).unwrap()
}

#[test]
fn the_urls() {
    assert!(URL.ends_with("/ws/v4/"));
    assert!(REST_URL.starts_with("https://"));
}

#[test]
fn subscribes_trades_in_one_message_and_books_one_each() {
    let subs = gate().subscriptions();
    assert_eq!(subs.len(), 2);
    let trades = std::str::from_utf8(&subs[0]).unwrap();
    assert!(trades.starts_with(r#"{"time":"#), "{trades}");
    assert!(trades.ends_with(r#","channel":"spot.trades","event":"subscribe","payload":["BTC_USDT"]}"#), "{trades}");
    let book = std::str::from_utf8(&subs[1]).unwrap();
    assert!(book.ends_with(r#","channel":"spot.order_book_update","event":"subscribe","payload":["BTC_USDT","100ms"]}"#), "{book}");
}

#[test]
fn data_frames_route_by_channel() {
    let mut r = gate();
    let mut sink = Collect::new();
    assert_eq!(r.route(TRADE, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::Trade));
    assert_eq!(sink.last().header.venue(), Ok(Venue::GateSpot));
    assert_eq!(r.route(ORDER_BOOK_UPDATE, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::SnapshotDelta));
}

#[test]
fn an_acknowledgement_and_a_pong_route_as_nothing() {
    let mut r = gate();
    let mut sink = Collect::new();
    assert_eq!(r.route(SUBSCRIBE_ACK, RECV_NS, &mut sink).unwrap(), 0);
    let pong = br#"{"time":1606294781,"time_ms":1606294781236,"channel":"spot.pong","event":"","result":null}"#;
    assert_eq!(r.route(pong, RECV_NS, &mut sink).unwrap(), 0);
    let other = String::from_utf8(TRADE.to_vec()).unwrap().replace("BTC_USDT", "ETH_USDT");
    assert_eq!(r.route(other.as_bytes(), RECV_NS, &mut sink).unwrap(), 0);
    assert_eq!(sink.len(), 0);
}

#[test]
fn the_start_book_is_a_get_with_id_and_decodes_to_a_quote() {
    let mut r = gate();
    let books = r.rest_books();
    assert_eq!(books.len(), 1);
    assert_eq!(books[0].instrument, 0);
    assert_eq!(books[0].request.method, Method::Get);
    assert_eq!(
        books[0].request.url,
        "https://api.gateio.ws/api/v4/spot/order_book?currency_pair=BTC_USDT&limit=10&with_id=true"
    );

    let mut sink = Collect::new();
    assert_eq!(r.route_rest(0, REST_ORDER_BOOK, RECV_NS, &mut sink).unwrap(), 1);
    let rec = sink.last();
    assert_eq!(rec.kind(), Ok(WireKind::Quote));
    let quote = rec.quote().unwrap();
    assert_eq!(quote.quote_ext_kind, quote_ext::SEQUENCE);
    assert_eq!(quote.quote_ext, 48776300);
}

#[test]
fn rest_can_be_pointed_elsewhere() {
    let r = gate().with_rest("http://127.0.0.1:8080/");
    assert!(r.rest_books()[0].request.url.starts_with("http://127.0.0.1:8080/api/v4/"));
}

#[test]
fn a_feed_without_delta_fetches_nothing() {
    let r = Router::new(vec![Subscription::new(btc_usdt(), ChannelSet::EMPTY.with(Channel::Trade))]).unwrap();
    assert!(r.rest_books().is_empty());
}

#[test]
fn the_ping_carries_the_time() {
    let mut r = gate();
    let mut out = [0u8; MAX_KEEPALIVE_LEN];
    let n = r.keepalive(1_606_294_781_000_000_000, &mut out).expect("due at once");
    assert_eq!(&out[..n], br#"{"time":1606294781,"channel":"spot.ping"}"#);
    assert_eq!(r.keepalive(1_606_294_782_000_000_000, &mut out), None);
}

#[test]
fn book_and_bbo_are_refused() {
    let e = Router::new(vec![Subscription::new(btc_usdt(), ChannelSet::EMPTY.with(Channel::Book))]).unwrap_err();
    assert_eq!(e, RouterError::Unsupported { venue: Venue::GateSpot, channel: Channel::Book });
}
