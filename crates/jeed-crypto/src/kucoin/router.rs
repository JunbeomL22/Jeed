//! KuCoin's [`Router`]: `topic`, the `bullet-public` ticket, and the REST
//! start book.
//!
//! ```json
//! {"type":"message","topic":"/market/level2:BTC-USDT","subject":"trade.l2update","data":{…}}
//! ```
//!
//! ## The address comes from a ticket
//!
//! KuCoin does not publish a WebSocket URL. `POST /api/v1/bullet-public`
//! answers with a server list, a token, and a ping cadence; the socket is
//! `{endpoint}?token={token}&connectId={id}`, and the token expires. So
//! [`ticket`](crate::recv::Router::ticket) names the request and
//! [`endpoint_from_ticket`](crate::recv::Router::endpoint_from_ticket) reads
//! the body — the binary makes the call before every connection attempt,
//! and the router keeps `pingInterval` for its keepalive. Until a ticket has
//! been read there is no endpoint, and a feed started while KuCoin's REST is
//! down waits on it like any other reconnect.
//!
//! ## Routing on `topic`
//!
//! `type` is `message` on data and `welcome` / `ack` / `pong` otherwise;
//! `topic` is `/channel:SYMBOL`. Both markets use the same envelope with
//! nothing shared inside it ([`kucoin`]), so the router
//! holds a [`Market`] and picks the decoder set once.
//!
//! ## Channels
//!
//! | conf | spot | futures |
//! |---|---|---|
//! | `trade` | `/market/match` | `/contractMarket/execution` |
//! | `delta` | `/market/level2` | `/contractMarket/level2` |
//!
//! Neither market has a whole-book channel; `delta` feeds get their start
//! book over REST after every open — `level2_100` on spot (the full-depth
//! endpoint needs an API key; the wire carries ten levels either way) and
//! `level2/snapshot` on futures.
//!
//! [`Router`]: crate::recv::Router

use crate::error::CryptoError;
use crate::json::{next_field, object_at, objects_at, parse_scalar_u64, parse_string_bytes, skip_value};
use crate::kucoin::{self, topic_symbol};
use crate::recv::endpoint::Endpoint;
use crate::recv::pipeline::MAX_KEEPALIVE_LEN;
use crate::recv::route::{self, Channel, ChannelSet, HttpRequest, RestBook, RouterError, Subscription, lookup};
use jeed_wire::{RecordSink, UnixNano, Venue};

/// Spot's REST base.
pub const SPOT_REST_URL: &str = "https://api.kucoin.com";

/// Futures' REST base.
pub const FUTURES_REST_URL: &str = "https://api-futures.kucoin.com";

/// The ping cadence assumed before a ticket has said otherwise.
pub const DEFAULT_PING_INTERVAL_NS: u64 = 18_000_000_000;

/// Which KuCoin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Market {
    /// [`Venue::KucoinSpot`].
    Spot,

    /// [`Venue::KucoinFutures`].
    Futures,
}

impl Market {
    /// The venue byte.
    #[inline]
    pub const fn venue(self) -> Venue {
        match self {
            Self::Spot => Venue::KucoinSpot,
            Self::Futures => Venue::KucoinFutures,
        }
    }

    /// The REST base.
    #[inline]
    pub const fn rest_url(self) -> &'static str {
        match self {
            Self::Spot => SPOT_REST_URL,
            Self::Futures => FUTURES_REST_URL,
        }
    }

    const fn trade_topic(self) -> &'static str {
        match self {
            Self::Spot => "/market/match",
            Self::Futures => "/contractMarket/execution",
        }
    }

    const fn delta_topic(self) -> &'static str {
        match self {
            Self::Spot => "/market/level2",
            Self::Futures => "/contractMarket/level2",
        }
    }
}

/// One KuCoin connection's instruments.
#[derive(Debug, Clone)]
pub struct Router {
    market: Market,
    subs: Vec<Subscription>,
    rest: String,
    ping_interval_ns: u64,
    next_ping_ns: UnixNano,
    /// Message ids, which KuCoin requires and echoes.
    next_id: u64,
}

const SUPPORTED: ChannelSet = ChannelSet::EMPTY.with(Channel::Trade).with(Channel::Delta);

impl Router {
    /// Builds a router for `market`. `bbo`, `book` and any `depth` are
    /// refused.
    pub fn new(market: Market, subs: Vec<Subscription>) -> Result<Self, RouterError> {
        route::check(market.venue(), &subs, SUPPORTED, |_| false)?;
        Ok(Self {
            market,
            subs,
            rest: market.rest_url().to_owned(),
            ping_interval_ns: DEFAULT_PING_INTERVAL_NS,
            next_ping_ns: 0,
            next_id: 1,
        })
    }

    /// The same, with REST — the ticket and the start books — at `base`
    /// (`https://host[:port]`) instead of the market's.
    #[must_use]
    pub fn with_rest(mut self, base: &str) -> Self {
        self.rest = base.trim_end_matches('/').to_owned();
        self
    }

    /// The market.
    #[inline]
    pub const fn market(&self) -> Market {
        self.market
    }

    /// The subscriptions, in conf order.
    #[inline]
    pub fn subscriptions_list(&self) -> &[Subscription] {
        &self.subs
    }

    /// The ping cadence in force — the ticket's, or the default.
    #[inline]
    pub const fn ping_interval_ns(&self) -> u64 {
        self.ping_interval_ns
    }

    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// The symbols wanting `channel`, comma-joined as a topic takes them.
    fn symbols(&self, channel: Channel) -> String {
        self.subs
            .iter()
            .filter(|s| s.channels.contains(channel))
            .map(|s| String::from_utf8_lossy(s.instrument.symbol_bytes()).into_owned())
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// `token`, `instanceServers[0].endpoint` and `pingInterval` from a
/// `bullet-public` body.
fn read_ticket(body: &[u8]) -> Result<(String, String, Option<u64>), RouterError> {
    let (data, _) = {
        let mut found = None;
        let mut pos = 0;
        while pos < body.len() {
            let (key, next) = next_field(body, pos);
            if key.is_empty() {
                break;
            }
            if key == b"data" {
                found = object_at(body, next);
                break;
            }
            pos = skip_value(body, next);
        }
        found.ok_or(RouterError::Ticket("no `data` object"))?
    };

    let mut token: Option<&[u8]> = None;
    let mut endpoint: Option<&[u8]> = None;
    let mut ping: Option<u64> = None;
    let mut pos = 0;
    while pos < data.len() {
        let (key, next) = next_field(data, pos);
        if key.is_empty() {
            break;
        }
        match key {
            b"token" => {
                let (s, p) = parse_string_bytes(data, next);
                token = Some(s);
                pos = p;
            }
            b"instanceServers" => {
                let mut servers = objects_at(data, next).ok_or(RouterError::Ticket("`instanceServers` is not an array"))?;
                let first = servers.next().ok_or(RouterError::Ticket("`instanceServers` is empty"))?;
                let mut p = 0;
                while p < first.len() {
                    let (k, n) = next_field(first, p);
                    if k.is_empty() {
                        break;
                    }
                    match k {
                        b"endpoint" => {
                            let (s, q) = parse_string_bytes(first, n);
                            endpoint = Some(s);
                            p = q;
                        }
                        b"pingInterval" => {
                            let (v, q) = parse_scalar_u64(first, n);
                            ping = Some(v);
                            p = q;
                        }
                        _ => p = skip_value(first, n),
                    }
                }
                pos = skip_value(data, next);
            }
            _ => pos = skip_value(data, next),
        }
    }

    let token = token.filter(|t| !t.is_empty()).ok_or(RouterError::Ticket("no `token`"))?;
    let endpoint = endpoint.filter(|e| !e.is_empty()).ok_or(RouterError::Ticket("no `endpoint`"))?;
    Ok((
        String::from_utf8_lossy(token).into_owned(),
        String::from_utf8_lossy(endpoint).into_owned(),
        ping,
    ))
}

impl crate::recv::Router for Router {
    fn venue(&self) -> Venue {
        self.market.venue()
    }

    fn route<S: RecordSink>(
        &mut self,
        msg: &[u8],
        recv_ns: UnixNano,
        sink: &mut S,
    ) -> Result<usize, CryptoError> {
        let mut topic: Option<&[u8]> = None;
        let mut pos = 0;
        while pos < msg.len() {
            let (key, next) = next_field(msg, pos);
            if key.is_empty() {
                break;
            }
            match key {
                b"type" => {
                    let (s, p) = parse_string_bytes(msg, next);
                    // `welcome`, `ack`, `pong`, `error`.
                    if s != b"message" {
                        return Ok(0);
                    }
                    pos = p;
                }
                b"topic" => {
                    let (s, p) = parse_string_bytes(msg, next);
                    topic = Some(s);
                    pos = p;
                }
                _ => pos = skip_value(msg, next),
            }
        }

        let Some(topic) = topic else {
            return Ok(0);
        };
        let Some(symbol) = topic_symbol(topic) else {
            return Ok(0);
        };
        let Some((_, sub)) = lookup(&self.subs, symbol) else {
            return Ok(0);
        };
        let inst = &sub.instrument;
        let channel = &topic[..topic.len() - symbol.len() - 1];

        match (self.market, channel) {
            (Market::Spot, b"/market/match") => {
                sink.publish(|rec| kucoin::spot::trade::decode(inst, msg, recv_ns, rec))?
            }
            (Market::Spot, b"/market/level2") => {
                sink.publish(|rec| kucoin::spot::delta::decode(inst, msg, recv_ns, rec))?
            }
            (Market::Futures, b"/contractMarket/execution") => {
                sink.publish(|rec| kucoin::futures::trade::decode(inst, msg, recv_ns, rec))?
            }
            (Market::Futures, b"/contractMarket/level2") => {
                sink.publish(|rec| kucoin::futures::delta::decode(inst, msg, recv_ns, rec))?
            }
            _ => return Ok(0),
        }
        Ok(1)
    }

    fn keepalive(&mut self, now: UnixNano, out: &mut [u8; MAX_KEEPALIVE_LEN]) -> Option<usize> {
        if now < self.next_ping_ns {
            return None;
        }
        self.next_ping_ns = now.saturating_add(self.ping_interval_ns);
        let id = self.take_id();
        let msg = format!(r#"{{"id":"{id}","type":"ping"}}"#);
        out[..msg.len()].copy_from_slice(msg.as_bytes());
        Some(msg.len())
    }

    fn subscriptions(&self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for (channel, topic) in
            [(Channel::Trade, self.market.trade_topic()), (Channel::Delta, self.market.delta_topic())]
        {
            let symbols = self.symbols(channel);
            if symbols.is_empty() {
                continue;
            }
            // Ids need only be unique per connection; the keepalive's counter
            // is not shared here because this takes `&self`.
            let id = out.len() as u64 + 1_000_000;
            out.push(
                format!(
                    r#"{{"id":"{id}","type":"subscribe","topic":"{topic}:{symbols}","privateChannel":false,"response":true}}"#
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
            .map(|(i, s)| {
                let symbol = String::from_utf8_lossy(s.instrument.symbol_bytes());
                let url = match self.market {
                    Market::Spot => {
                        format!("{}/api/v1/market/orderbook/level2_100?symbol={symbol}", self.rest)
                    }
                    Market::Futures => format!("{}/api/v1/level2/snapshot?symbol={symbol}", self.rest),
                };
                RestBook { instrument: i, request: HttpRequest::get(url) }
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
        match self.market {
            Market::Spot => sink.publish(|rec| kucoin::spot::snapshot::decode(inst, body, recv_ns, rec))?,
            Market::Futures => {
                sink.publish(|rec| kucoin::futures::snapshot::decode(inst, body, recv_ns, rec))?
            }
        }
        Ok(1)
    }

    fn ticket(&self) -> Option<HttpRequest> {
        Some(HttpRequest::post(format!("{}/api/v1/bullet-public", self.rest)))
    }

    fn endpoint_from_ticket(&mut self, body: &[u8]) -> Result<Endpoint, RouterError> {
        let (token, endpoint, ping_ms) = read_ticket(body)?;
        if let Some(ms) = ping_ms.filter(|&ms| ms > 0) {
            self.ping_interval_ns = ms * 1_000_000;
        }
        let id = self.take_id();
        let sep = if endpoint.contains('?') { '&' } else { '?' };
        format!("{endpoint}{sep}token={token}&connectId=jeed{id}")
            .parse()
            .map_err(|_| RouterError::Ticket("`endpoint` is not a WebSocket URL"))
    }
}
