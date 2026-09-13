//! Bybit's [`Router`]: `topic`, `op: subscribe`, and `{"op":"ping"}`.
//!
//! ```json
//! {"topic":"orderbook.50.BTCUSDT","type":"delta","ts":…,"data":{…}}
//! ```
//!
//! ## Routing on `topic`
//!
//! `publicTrade.BTCUSDT` and `orderbook.50.BTCUSDT`: the channel is the
//! first dotted segment and the symbol the last. A frame with no `topic` —
//! `{"success":true,"ret_msg":"subscribe",…}`, or the pong
//! `{"success":true,"ret_msg":"pong","op":"ping",…}` — is conversation and
//! routes as nothing.
//!
//! ## One router, two venues
//!
//! Spot and linear speak the same protocol on different URLs, and the
//! records differ only in the venue byte the [`Instrument`](crate::Instrument)
//! carries. [`Router::new`] takes the venue and holds every instrument to it.
//!
//! ## Depth
//!
//! The `book` channel is `orderbook.{depth}`, a whole book then diffs; the
//! decoder tells the two apart per frame. Depth 1 is best bid and ask; 50 is
//! the default. Spot offers 1, 50, 200 and 1000; linear 1, 50, 200, 500 and
//! 1000.
//!
//! ## `{"op":"ping"}` every twenty seconds
//!
//! Bybit asks for one and disconnects without it. Sent on a schedule.
//!
//! [`Router`]: crate::recv::Router

use crate::bybit;
use crate::error::CryptoError;
use crate::json::{next_field, parse_string_bytes, skip_value};
use crate::recv::pipeline::MAX_KEEPALIVE_LEN;
use crate::recv::route::{self, Channel, ChannelSet, RouterError, Subscription, lookup, publish_batch};
use jeed_wire::{RecordSink, UnixNano, Venue};

/// Spot's public WebSocket.
pub const SPOT_URL: &str = "wss://stream.bybit.com/v5/public/spot";

/// Linear perpetuals' public WebSocket.
pub const LINEAR_URL: &str = "wss://stream.bybit.com/v5/public/linear";

/// Depth the `book` channel takes when the conf names none.
pub const DEFAULT_DEPTH: u16 = 50;

/// How often to send `{"op":"ping"}`.
pub const PING_INTERVAL_NS: u64 = 20_000_000_000;

/// The venue keepalive.
pub const PING: &[u8] = br#"{"op":"ping"}"#;

/// The default URL for `venue`, or `None` if it is not one of the two.
pub const fn url(venue: Venue) -> Option<&'static str> {
    match venue {
        Venue::BybitSpot => Some(SPOT_URL),
        Venue::BybitLinear => Some(LINEAR_URL),
        _ => None,
    }
}

/// One Bybit connection's instruments.
#[derive(Debug, Clone)]
pub struct Router {
    venue: Venue,
    subs: Vec<Subscription>,
    next_ping_ns: UnixNano,
}

const SUPPORTED: ChannelSet = ChannelSet::EMPTY.with(Channel::Trade).with(Channel::Book);

impl Router {
    /// Builds a router for [`Venue::BybitSpot`] or [`Venue::BybitLinear`].
    pub fn new(venue: Venue, subs: Vec<Subscription>) -> Result<Self, RouterError> {
        if url(venue).is_none() {
            return Err(RouterError::NotCrypto(venue));
        }
        let depth_ok = move |d: u16| match venue {
            Venue::BybitSpot => matches!(d, 1 | 50 | 200 | 1000),
            _ => matches!(d, 1 | 50 | 200 | 500 | 1000),
        };
        route::check(venue, &subs, SUPPORTED, depth_ok)?;
        Ok(Self { venue, subs, next_ping_ns: 0 })
    }

    /// The subscriptions, in conf order.
    #[inline]
    pub fn subscriptions_list(&self) -> &[Subscription] {
        &self.subs
    }

    /// The topics this router subscribes to.
    pub fn topics(&self) -> Vec<String> {
        let mut out = Vec::new();
        for sub in &self.subs {
            let symbol = String::from_utf8_lossy(sub.instrument.symbol_bytes());
            if sub.channels.contains(Channel::Trade) {
                out.push(format!("publicTrade.{symbol}"));
            }
            if sub.channels.contains(Channel::Book) {
                out.push(format!("orderbook.{}.{symbol}", sub.depth.unwrap_or(DEFAULT_DEPTH)));
            }
        }
        out
    }
}

/// `orderbook.50.BTCUSDT` → `(b"orderbook", b"BTCUSDT")`.
fn split_topic(topic: &[u8]) -> Option<(&[u8], &[u8])> {
    let first = topic.iter().position(|&b| b == b'.')?;
    let last = topic.iter().rposition(|&b| b == b'.')?;
    Some((&topic[..first], &topic[last + 1..]))
}

impl crate::recv::Router for Router {
    fn venue(&self) -> Venue {
        self.venue
    }

    fn route<S: RecordSink>(
        &mut self,
        msg: &[u8],
        recv_ns: UnixNano,
        sink: &mut S,
    ) -> Result<usize, CryptoError> {
        let mut topic: Option<&[u8]> = None;
        let mut pos = 0;
        while pos < msg.len() && topic.is_none() {
            let (key, next) = next_field(msg, pos);
            if key.is_empty() {
                break;
            }
            if key == b"topic" {
                let (s, p) = parse_string_bytes(msg, next);
                topic = Some(s);
                pos = p;
            } else {
                pos = skip_value(msg, next);
            }
        }

        let Some(topic) = topic else {
            return Ok(0);
        };
        let Some((channel, symbol)) = split_topic(topic) else {
            return Ok(0);
        };
        let Some((_, sub)) = lookup(&self.subs, symbol) else {
            return Ok(0);
        };
        let inst = &sub.instrument;

        match channel {
            b"publicTrade" => {
                let mut prints = bybit::trade::trades(msg)?;
                publish_batch(sink, |rec| prints.decode_next(inst, recv_ns, rec))
            }
            b"orderbook" => {
                sink.publish(|rec| bybit::book::decode(inst, msg, recv_ns, rec))?;
                Ok(1)
            }
            _ => Ok(0),
        }
    }

    fn keepalive(&mut self, now: UnixNano, out: &mut [u8; MAX_KEEPALIVE_LEN]) -> Option<usize> {
        if now < self.next_ping_ns {
            return None;
        }
        self.next_ping_ns = now.saturating_add(PING_INTERVAL_NS);
        out[..PING.len()].copy_from_slice(PING);
        Some(PING.len())
    }

    fn subscriptions(&self) -> Vec<Vec<u8>> {
        // Ten args per request is the documented limit on spot.
        self.topics()
            .chunks(10)
            .map(|chunk| {
                let mut msg = String::from(r#"{"op":"subscribe","args":["#);
                for (i, topic) in chunk.iter().enumerate() {
                    if i > 0 {
                        msg.push(',');
                    }
                    msg.push('"');
                    msg.push_str(topic);
                    msg.push('"');
                }
                msg.push_str("]}");
                msg.into_bytes()
            })
            .collect()
    }
}
