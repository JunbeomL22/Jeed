//! Binance's [`Router`]: the combined-stream envelope, and `SUBSCRIBE`.
//!
//! ```json
//! {"stream":"btcusdt@trade","data":{"e":"trade","s":"BTCUSDT",…}}
//! ```
//!
//! ## Combined streams, always
//!
//! Binance offers a URL per stream (`/ws/btcusdt@trade`) and a multiplexed
//! one (`/stream`) on which each message says which stream it belongs to.
//! One connection per feed is the design, so it is the latter, and the
//! router's job is the envelope: read `stream`, cut the `data` object out,
//! and hand the inner bytes — a flat object, exactly what the decoders
//! expect — to the right one.
//!
//! The stream name is `<symbol lower-case>@<suffix>`, so the symbol is kept
//! lower-cased once at build time and compared without a case fold per
//! message.
//!
//! ## Subscribing is one message
//!
//! `{"method":"SUBSCRIBE","params":[…],"id":1}` after every open, answered
//! by `{"result":null,"id":1}` — a message with no `stream`, routed as
//! nothing. Listing the streams in the URL instead would work too, and would
//! tie the URL to the conf; a message keeps the endpoint constant.
//!
//! ## Nothing to keep alive
//!
//! Binance pings first (spot every twenty seconds, USD-M every three
//! minutes) and the loop answers at the protocol layer. No venue-level ping.
//!
//! [`Router`]: crate::recv::Router

use crate::binance::{futures, spot};
use crate::error::CryptoError;
use crate::json::{next_field, object_at, parse_string_bytes, skip_value};
use crate::recv::route::{self, Channel, ChannelSet, RouterError, Subscription};
use jeed_wire::{RecordSink, UnixNano, Venue};

/// Spot's combined-stream URL.
pub const SPOT_URL: &str = "wss://stream.binance.com:9443/stream";

/// USD-M's combined-stream URL.
pub const FUTURES_URL: &str = "wss://fstream.binance.com/stream";

/// Depth the `book` channel takes when the conf names none.
pub const DEFAULT_DEPTH: u16 = 10;

/// Which Binance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Market {
    /// `stream.binance.com` — [`Venue::BinanceSpot`].
    Spot,

    /// `fstream.binance.com` — [`Venue::BinanceFutures`].
    Futures,
}

impl Market {
    /// The venue byte.
    #[inline]
    pub const fn venue(self) -> Venue {
        match self {
            Self::Spot => Venue::BinanceSpot,
            Self::Futures => Venue::BinanceFutures,
        }
    }

    /// The default URL.
    #[inline]
    pub const fn url(self) -> &'static str {
        match self {
            Self::Spot => SPOT_URL,
            Self::Futures => FUTURES_URL,
        }
    }
}

/// One Binance connection's instruments.
#[derive(Debug, Clone)]
pub struct Router {
    market: Market,
    subs: Vec<Subscription>,
    /// Each subscription's symbol as it appears in a stream name.
    lower: Vec<Vec<u8>>,
}

const SUPPORTED: ChannelSet = ChannelSet::EMPTY
    .with(Channel::Trade)
    .with(Channel::Bbo)
    .with(Channel::Book)
    .with(Channel::Delta);

impl Router {
    /// Builds a router for `market`, refusing what Binance cannot serve.
    ///
    /// Every channel is available; the book depth must be 5, 10 or 20.
    pub fn new(market: Market, subs: Vec<Subscription>) -> Result<Self, RouterError> {
        route::check(market.venue(), &subs, SUPPORTED, |d| matches!(d, 5 | 10 | 20))?;
        let lower = subs.iter().map(|s| s.instrument.symbol_bytes().to_ascii_lowercase()).collect();
        Ok(Self { market, subs, lower })
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

    /// The stream names this router asks for, in `params` order.
    pub fn streams(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (sub, lower) in self.subs.iter().zip(&self.lower) {
            let symbol = String::from_utf8_lossy(lower);
            for channel in sub.channels.iter() {
                let suffix = match (channel, self.market) {
                    (Channel::Trade, Market::Spot) => "trade".to_owned(),
                    (Channel::Trade, Market::Futures) => "aggTrade".to_owned(),
                    (Channel::Bbo, _) => "bookTicker".to_owned(),
                    (Channel::Book, _) => format!("depth{}@100ms", sub.depth.unwrap_or(DEFAULT_DEPTH)),
                    (Channel::Delta, _) => "depth@100ms".to_owned(),
                };
                out.push(format!("{symbol}@{suffix}"));
            }
        }
        out
    }
}

/// What a stream suffix names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stream {
    Trade,
    Bbo,
    Snapshot,
    Delta,
}

/// `btcusdt@depth10@100ms` → `(b"btcusdt", Snapshot)`.
fn split_stream(name: &[u8]) -> Option<(&[u8], Stream)> {
    let at = name.iter().position(|&b| b == b'@')?;
    let (symbol, suffix) = (&name[..at], &name[at + 1..]);
    let stream = match suffix {
        b"trade" | b"aggTrade" => Stream::Trade,
        b"bookTicker" => Stream::Bbo,
        b"depth" | b"depth@100ms" | b"depth@250ms" | b"depth@500ms" => Stream::Delta,
        s if s.starts_with(b"depth") => Stream::Snapshot,
        _ => return None,
    };
    Some((symbol, stream))
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
        let mut stream: Option<&[u8]> = None;
        let mut data: Option<&[u8]> = None;

        let mut pos = 0;
        while pos < msg.len() {
            let (key, next) = next_field(msg, pos);
            if key.is_empty() {
                break;
            }
            match key {
                b"stream" => {
                    let (s, p) = parse_string_bytes(msg, next);
                    stream = Some(s);
                    pos = p;
                }
                b"data" => match object_at(msg, next) {
                    Some((inner, p)) => {
                        data = Some(inner);
                        pos = p;
                    }
                    None => pos = skip_value(msg, next),
                },
                _ => pos = skip_value(msg, next),
            }
        }

        // No envelope: a `{"result":null,"id":1}` acknowledgement, or an
        // `{"error":…}` — neither is a stream this router owns.
        let (Some(stream), Some(data)) = (stream, data) else {
            return Ok(0);
        };
        let Some((symbol, kind)) = split_stream(stream) else {
            return Ok(0);
        };
        let Some(i) = self.lower.iter().position(|l| l.as_slice() == symbol) else {
            return Ok(0);
        };
        let inst = &self.subs[i].instrument;

        match (self.market, kind) {
            (Market::Spot, Stream::Trade) => sink.publish(|rec| spot::trade::decode(inst, data, recv_ns, rec))?,
            (Market::Spot, Stream::Bbo) => sink.publish(|rec| spot::bbo::decode(inst, data, recv_ns, rec))?,
            (Market::Spot, Stream::Snapshot) => {
                sink.publish(|rec| spot::snapshot::decode(inst, data, recv_ns, rec))?
            }
            (Market::Spot, Stream::Delta) => sink.publish(|rec| spot::delta::decode(inst, data, recv_ns, rec))?,
            (Market::Futures, Stream::Trade) => {
                sink.publish(|rec| futures::trade::decode(inst, data, recv_ns, rec))?
            }
            (Market::Futures, Stream::Bbo) => sink.publish(|rec| futures::bbo::decode(inst, data, recv_ns, rec))?,
            (Market::Futures, Stream::Snapshot) => {
                sink.publish(|rec| futures::snapshot::decode(inst, data, recv_ns, rec))?
            }
            (Market::Futures, Stream::Delta) => {
                sink.publish(|rec| futures::delta::decode(inst, data, recv_ns, rec))?
            }
        }
        Ok(1)
    }

    fn subscriptions(&self) -> Vec<Vec<u8>> {
        let mut msg = String::from(r#"{"method":"SUBSCRIBE","params":["#);
        for (i, stream) in self.streams().iter().enumerate() {
            if i > 0 {
                msg.push(',');
            }
            msg.push('"');
            msg.push_str(stream);
            msg.push('"');
        }
        msg.push_str(r#"],"id":1}"#);
        vec![msg.into_bytes()]
    }
}
