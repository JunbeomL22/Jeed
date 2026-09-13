//! Tests for `src/kucoin/router.rs` — `topic`, the `bullet-public` ticket,
//! and the REST start book.

#[allow(dead_code)]
#[path = "../recv/sink.rs"]
mod sink;

use crate::common::{
    FUTURES_EXECUTION, FUTURES_LEVEL2, FUTURES_REST, RECV_NS, SPOT_LEVEL2, SPOT_MATCH, SPOT_REST,
    futures_xbtusdtm, spot_btc_usdt,
};
use jeed_crypto::Instrument;
use jeed_crypto::kucoin::router::{DEFAULT_PING_INTERVAL_NS, Market, Router};
use jeed_crypto::recv::{Channel, ChannelSet, MAX_KEEPALIVE_LEN, Method, Router as _, RouterError, Subscription};
use jeed_wire::{Venue, WireKind, quote_ext};
use sink::Collect;

fn both() -> ChannelSet {
    ChannelSet::EMPTY.with(Channel::Trade).with(Channel::Delta)
}

fn spot() -> Router {
    Router::new(Market::Spot, vec![Subscription::new(spot_btc_usdt(), both())]).unwrap()
}

fn futures() -> Router {
    Router::new(Market::Futures, vec![Subscription::new(futures_xbtusdtm(), both())]).unwrap()
}

/// What `bullet-public` answers.
const TICKET: &[u8] = br#"{"code":"200000","data":{"token":"2neAiuYvAU61ZDXANAGAsiL4-iAExhsBXZxftpOeh_55i3Ysy2q2LEsEWU64mdzUOPusi34M_wGoSf7iNyEWJ1UQy47YbpY4zVdzilNP-Bj3iXzrjjGlWtiYB9J6i9GjsxUuhPw3BlrzazF6ghq4Lzf7w3lGuSjHQ1F3T6tBWNm3B6pqRBLLwNXhVTgMWJa1nI=.LM7xUTTnMUlCUXvKf0jBgw==","instanceServers":[{"endpoint":"wss://ws-api-spot.kucoin.com/","encrypt":true,"protocol":"websocket","pingInterval":18000,"pingTimeout":10000}]}}"#;

#[test]
fn the_ticket_is_a_post_to_the_markets_rest() {
    let t = spot().ticket().expect("kucoin needs a ticket");
    assert_eq!(t.method, Method::Post);
    assert_eq!(t.url, "https://api.kucoin.com/api/v1/bullet-public");
    let t = futures().ticket().unwrap();
    assert_eq!(t.url, "https://api-futures.kucoin.com/api/v1/bullet-public");
}

#[test]
fn the_ticket_body_names_the_endpoint_and_sets_the_ping_cadence() {
    let mut r = spot();
    assert_eq!(r.ping_interval_ns(), DEFAULT_PING_INTERVAL_NS);
    let ep = r.endpoint_from_ticket(TICKET).unwrap();
    assert!(ep.tls);
    assert_eq!(ep.host, "ws-api-spot.kucoin.com");
    assert_eq!(ep.port, 443);
    assert!(ep.path.starts_with("/?token=2neAiuYvAU61"), "{}", ep.path);
    assert!(ep.path.ends_with("&connectId=jeed1"), "{}", ep.path);
    assert_eq!(r.ping_interval_ns(), 18_000_000_000);

    // A second ticket gets a fresh connect id.
    let ep = r.endpoint_from_ticket(TICKET).unwrap();
    assert!(ep.path.ends_with("&connectId=jeed2"));
}

#[test]
fn a_ticket_without_the_parts_is_refused() {
    let mut r = spot();
    assert_eq!(r.endpoint_from_ticket(br#"{"code":"500000","msg":"down"}"#), Err(RouterError::Ticket("no `data` object")));
    assert_eq!(
        r.endpoint_from_ticket(br#"{"data":{"instanceServers":[{"endpoint":"wss://x/"}]}}"#),
        Err(RouterError::Ticket("no `token`"))
    );
    assert_eq!(
        r.endpoint_from_ticket(br#"{"data":{"token":"t","instanceServers":[]}}"#),
        Err(RouterError::Ticket("`instanceServers` is empty"))
    );
    assert_eq!(
        r.endpoint_from_ticket(br#"{"data":{"token":"t","instanceServers":[{"endpoint":"https://x/"}]}}"#),
        Err(RouterError::Ticket("`endpoint` is not a WebSocket URL"))
    );
}

#[test]
fn subscribes_one_topic_per_channel_with_the_symbols_joined() {
    let eth = Instrument::new(Venue::KucoinSpot, b"ETH-USDT", 2, 8).unwrap();
    let r = Router::new(
        Market::Spot,
        vec![Subscription::new(spot_btc_usdt(), both()), Subscription::new(eth, ChannelSet::EMPTY.with(Channel::Delta))],
    )
    .unwrap();
    let subs = r.subscriptions();
    assert_eq!(subs.len(), 2);
    assert_eq!(
        std::str::from_utf8(&subs[0]).unwrap(),
        r#"{"id":"1000000","type":"subscribe","topic":"/market/match:BTC-USDT","privateChannel":false,"response":true}"#
    );
    assert_eq!(
        std::str::from_utf8(&subs[1]).unwrap(),
        r#"{"id":"1000001","type":"subscribe","topic":"/market/level2:BTC-USDT,ETH-USDT","privateChannel":false,"response":true}"#
    );
    let subs = futures().subscriptions();
    assert!(std::str::from_utf8(&subs[0]).unwrap().contains(r#""topic":"/contractMarket/execution:XBTUSDTM""#));
    assert!(std::str::from_utf8(&subs[1]).unwrap().contains(r#""topic":"/contractMarket/level2:XBTUSDTM""#));
}

#[test]
fn spot_frames_route_by_topic() {
    let mut r = spot();
    let mut sink = Collect::new();
    assert_eq!(r.route(SPOT_MATCH, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::Trade));
    assert_eq!(sink.last().header.venue(), Ok(Venue::KucoinSpot));
    assert_eq!(r.route(SPOT_LEVEL2, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::SnapshotDelta));
}

#[test]
fn futures_frames_route_by_topic() {
    let mut r = futures();
    let mut sink = Collect::new();
    assert_eq!(r.route(FUTURES_EXECUTION, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::Trade));
    assert_eq!(sink.last().header.venue(), Ok(Venue::KucoinFutures));
    assert_eq!(r.route(FUTURES_LEVEL2, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::SnapshotDelta));
}

#[test]
fn a_spot_frame_on_a_futures_router_routes_as_nothing() {
    let mut r = futures();
    let mut sink = Collect::new();
    assert_eq!(r.route(SPOT_MATCH, RECV_NS, &mut sink).unwrap(), 0);
}

#[test]
fn conversation_routes_as_nothing() {
    let mut r = spot();
    let mut sink = Collect::new();
    assert_eq!(r.route(br#"{"id":"hQvf8jkno","type":"welcome"}"#, RECV_NS, &mut sink).unwrap(), 0);
    assert_eq!(r.route(br#"{"id":"1000000","type":"ack"}"#, RECV_NS, &mut sink).unwrap(), 0);
    assert_eq!(r.route(br#"{"id":"3","type":"pong"}"#, RECV_NS, &mut sink).unwrap(), 0);
    assert_eq!(sink.len(), 0);
}

#[test]
fn the_ping_carries_an_id_at_the_tickets_cadence() {
    let mut r = spot();
    let mut out = [0u8; MAX_KEEPALIVE_LEN];
    let n = r.keepalive(100, &mut out).expect("due at once");
    assert_eq!(&out[..n], br#"{"id":"1","type":"ping"}"#);
    assert_eq!(r.keepalive(100 + DEFAULT_PING_INTERVAL_NS - 1, &mut out), None);
    let n = r.keepalive(100 + DEFAULT_PING_INTERVAL_NS, &mut out).expect("due again");
    assert_eq!(&out[..n], br#"{"id":"2","type":"ping"}"#);
}

#[test]
fn start_books_are_the_public_endpoints_and_decode_to_quotes() {
    let mut r = spot();
    let books = r.rest_books();
    assert_eq!(books.len(), 1);
    assert_eq!(books[0].request.url, "https://api.kucoin.com/api/v1/market/orderbook/level2_100?symbol=BTC-USDT");
    let mut sink = Collect::new();
    assert_eq!(r.route_rest(0, SPOT_REST, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::Quote));
    assert_eq!(sink.last().quote().unwrap().quote_ext_kind, quote_ext::SEQUENCE);

    let mut r = futures();
    assert_eq!(r.rest_books()[0].request.url, "https://api-futures.kucoin.com/api/v1/level2/snapshot?symbol=XBTUSDTM");
    assert_eq!(r.route_rest(0, FUTURES_REST, RECV_NS, &mut sink).unwrap(), 1);
    assert_eq!(sink.last().kind(), Ok(WireKind::Quote));
    assert_eq!(sink.last().header.venue(), Ok(Venue::KucoinFutures));
}

#[test]
fn rest_can_be_pointed_elsewhere_for_ticket_and_books() {
    let r = spot().with_rest("http://127.0.0.1:9000");
    assert_eq!(r.ticket().unwrap().url, "http://127.0.0.1:9000/api/v1/bullet-public");
    assert!(r.rest_books()[0].request.url.starts_with("http://127.0.0.1:9000/api/v1/market/"));
}

#[test]
fn book_bbo_and_depth_are_refused() {
    let e = Router::new(Market::Spot, vec![Subscription::new(spot_btc_usdt(), ChannelSet::EMPTY.with(Channel::Book))]).unwrap_err();
    assert_eq!(e, RouterError::Unsupported { venue: Venue::KucoinSpot, channel: Channel::Book });
    let e = Router::new(Market::Futures, vec![Subscription::new(spot_btc_usdt(), both())]).unwrap_err();
    assert_eq!(e, RouterError::WrongVenue { expected: Venue::KucoinFutures, found: Venue::KucoinSpot });
}
