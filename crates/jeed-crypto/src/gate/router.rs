//! Gate's [`Router`]: `channel` + `event`, timed subscriptions, and the
//! REST start book.
//!
//! ```json
//! {"time":…,"channel":"spot.order_book_update","event":"update","result":{"s":"BTC_USDT",…}}
//! ```
//!
//! ## Routing on `channel`, gated by `event`
//!
//! Every frame names its `channel`; `event` is `update` on data and
//! `subscribe` / `unsubscribe` on the answers to ours, which carry a
//! `result` of `{"status":"success"}` rather than a book. So `event` is
//! read first and anything but `update` routes as nothing. The symbol is
//! inside `result` — `s` on the book channel, `currency_pair` on trades —
//! so the router looks one level in for it.
//!
//! ## Channels
//!
//! | conf | Gate |
//! |---|---|
//! | `trade` | `spot.trades` — one print per frame |
//! | `delta` | `spot.order_book_update` at `100ms` — diffs only |
//!
//! There is no whole-book channel. A feed that asks for `delta` gets its
//! start book from `/api/v4/spot/order_book` after every open
//! ([`rest_books`](crate::recv::Router::rest_books)), with `with_id=true` so
//! the body carries the `id` the diffs chain from
//! ([`gate::snapshot`]). `book` is refused: there is
//! nothing on the socket to subscribe to.
//!
//! ## Every message carries the time
//!
//! Gate wants a `time` (seconds) on subscriptions and pings. The router
//! reads the clock for it — the only router that does — because a
//! subscription with yesterday's time is refused.
//!
//! [`Router`]: crate::recv::Router

use crate::clock;
use crate::error::CryptoError;
use crate::gate;
use crate::json::{next_field, object_at, parse_string_bytes, skip_value};
use crate::recv::pipeline::MAX_KEEPALIVE_LEN;
use crate::recv::route::{self, Channel, ChannelSet, HttpRequest, RestBook, RouterError, Subscription, lookup};
use jeed_wire::{RecordSink, UnixNano, Venue};

/// The public WebSocket.
pub const URL: &str = "wss://api.gateio.ws/ws/v4/";

/// The REST base the start book is fetched from.
pub const REST_URL: &str = "https://api.gateio.ws";

/// How often to send `spot.ping`.
pub const PING_INTERVAL_NS: u64 = 20_000_000_000;

/// Levels asked of the REST start book. The wire carries ten.
pub const REST_LEVELS: u16 = 10;

/// One Gate connection's instruments.
#[derive(Debug, Clone)]
pub struct Router {
    subs: Vec<Subscription>,
    rest: String,
    next_ping_ns: UnixNano,
}

const SUPPORTED: ChannelSet = ChannelSet::EMPTY.with(Channel::Trade).with(Channel::Delta);

impl Router {
    /// Builds a router. `bbo`, `book` and any `depth` are refused.
    pub fn new(subs: Vec<Subscription>) -> Result<Self, RouterError> {
        route::check(Venue::GateSpot, &subs, SUPPORTED, |_| false)?;
        Ok(Self { subs, rest: REST_URL.to_owned(), next_ping_ns: 0 })
    }

    /// The same, fetching start books from `base` (`https://host[:port]`)
    /// instead of [`REST_URL`] — a proxy, or a test.
    #[must_use]
    pub fn with_rest(mut self, base: &str) -> Self {
        self.rest = base.trim_end_matches('/').to_owned();
        self
    }

    /// The subscriptions, in conf order.
    #[inline]
    pub fn subscriptions_list(&self) -> &[Subscription] {
        &self.subs
    }

    fn now_secs() -> u64 {
        clock::now_ns() / 1_000_000_000
    }
}

/// `result`'s `s` or `currency_pair`, whichever it has.
fn result_symbol(result: &[u8]) -> Option<&[u8]> {
    let mut pos = 0;
    while pos < result.len() {
        let (key, next) = next_field(result, pos);
        if key.is_empty() {
            break;
        }
        if key == b"s" || key == b"currency_pair" {
            return Some(parse_string_bytes(result, next).0);
        }
        pos = skip_value(result, next);
    }
    None
}

impl crate::recv::Router for Router {
    fn venue(&self) -> Venue {
        Venue::GateSpot
    }

    fn route<S: RecordSink>(
        &mut self,
        msg: &[u8],
        recv_ns: UnixNano,
        sink: &mut S,
    ) -> Result<usize, CryptoError> {
        let mut channel: Option<&[u8]> = None;
        let mut result: Option<&[u8]> = None;

        let mut pos = 0;
        while pos < msg.len() {
            let (key, next) = next_field(msg, pos);
            if key.is_empty() {
                break;
            }
            match key {
                b"channel" => {
                    let (s, p) = parse_string_bytes(msg, next);
                    channel = Some(s);
                    pos = p;
                }
                b"event" => {
                    let (s, p) = parse_string_bytes(msg, next);
                    if s != b"update" {
                        return Ok(0);
                    }
                    pos = p;
                }
                b"result" => match object_at(msg, next) {
                    Some((inner, p)) => {
                        result = Some(inner);
                        pos = p;
                    }
                    None => pos = skip_value(msg, next),
                },
                _ => pos = skip_value(msg, next),
            }
        }

        let (Some(channel), Some(result)) = (channel, result) else {
            return Ok(0);
        };
        let Some(symbol) = result_symbol(result) else {
            return Ok(0);
        };
        let Some((_, sub)) = lookup(&self.subs, symbol) else {
            return Ok(0);
        };
        let inst = &sub.instrument;

        match channel {
            b"spot.trades" => sink.publish(|rec| gate::trade::decode(inst, msg, recv_ns, rec))?,
            b"spot.order_book_update" => sink.publish(|rec| gate::delta::decode(inst, msg, recv_ns, rec))?,
            _ => return Ok(0),
        }
        Ok(1)
    }

    fn keepalive(&mut self, now: UnixNano, out: &mut [u8; MAX_KEEPALIVE_LEN]) -> Option<usize> {
        if now < self.next_ping_ns {
            return None;
        }
        self.next_ping_ns = now.saturating_add(PING_INTERVAL_NS);
        let msg = format!(r#"{{"time":{},"channel":"spot.ping"}}"#, now / 1_000_000_000);
        out[..msg.len()].copy_from_slice(msg.as_bytes());
        Some(msg.len())
    }

    fn subscriptions(&self) -> Vec<Vec<u8>> {
        let time = Self::now_secs();
        let mut out = Vec::new();

        // Trades take every pair in one payload.
        let pairs: Vec<String> = self
            .subs
            .iter()
            .filter(|s| s.channels.contains(Channel::Trade))
            .map(|s| format!("\"{}\"", String::from_utf8_lossy(s.instrument.symbol_bytes())))
            .collect();
        if !pairs.is_empty() {
            out.push(
                format!(
                    r#"{{"time":{time},"channel":"spot.trades","event":"subscribe","payload":[{}]}}"#,
                    pairs.join(",")
                )
                .into_bytes(),
            );
        }

        // The book channel's payload is `[pair, interval]`: one message each.
        for sub in self.subs.iter().filter(|s| s.channels.contains(Channel::Delta)) {
            let pair = String::from_utf8_lossy(sub.instrument.symbol_bytes());
            out.push(
                format!(
                    r#"{{"time":{time},"channel":"spot.order_book_update","event":"subscribe","payload":["{pair}","100ms"]}}"#
                )
                .into_bytes(),
            );
        }
        out
    }

    fn rest_books(&self) -> Vec<RestBook> {
        self.subs
            .iter()
            .enumerate()
            .filter(|(_, s)| s.channels.contains(Channel::Delta))
            .map(|(i, s)| RestBook {
                instrument: i,
                request: HttpRequest::get(format!(
                    "{}/api/v4/spot/order_book?currency_pair={}&limit={REST_LEVELS}&with_id=true",
                    self.rest,
                    String::from_utf8_lossy(s.instrument.symbol_bytes())
                )),
            })
            .collect()
    }

    fn route_rest<S: RecordSink>(
        &mut self,
        instrument: usize,
        body: &[u8],
        recv_ns: UnixNano,
        sink: &mut S,
    ) -> Result<usize, CryptoError> {
        let inst = &self.subs.get(instrument).ok_or(CryptoError::Empty)?.instrument;
        sink.publish(|rec| gate::snapshot::decode(inst, body, recv_ns, rec))?;
        Ok(1)
    }
}
