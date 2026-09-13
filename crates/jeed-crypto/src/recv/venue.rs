//! One [`Router`] over all of them, chosen by the venue byte.
//!
//! The receive loop is generic in its router, and a binary that reads the
//! venue from a conf needs one type it can name. [`Router::route`] takes a
//! generic sink, so the trait is not object-safe and this is an enum rather
//! than a `Box<dyn Router>`; every method matches once and delegates.
//!
//! Nothing here knows a venue's protocol. The knowledge is in each venue's
//! `router` module; this is the switchboard.

use crate::error::CryptoError;
use crate::recv::endpoint::Endpoint;
use crate::recv::pipeline::{MAX_KEEPALIVE_LEN, Router};
use crate::recv::route::{HttpRequest, RestBook, RouterError, Subscription};
use crate::{binance, bitget, bybit, gate, kucoin, okx, upbit};
use jeed_wire::{RecordSink, UnixNano, Venue};

/// Any crypto venue's router.
#[derive(Debug, Clone)]
pub enum VenueRouter {
    /// Binance spot or USD-M.
    Binance(binance::router::Router),

    /// Upbit or Bithumb.
    Upbit(upbit::router::Router),

    /// OKX.
    Okx(okx::router::Router),

    /// Bybit spot or linear.
    Bybit(bybit::router::Router),

    /// Bitget spot or USDT-futures.
    Bitget(bitget::router::Router),

    /// Gate spot.
    Gate(gate::router::Router),

    /// KuCoin spot or futures.
    Kucoin(kucoin::router::Router),
}

impl VenueRouter {
    /// Builds the router for `venue` from `subs`.
    ///
    /// Every instrument in `subs` must be on `venue`; the venue's own
    /// constructor says what else it refuses.
    pub fn new(venue: Venue, subs: Vec<Subscription>) -> Result<Self, RouterError> {
        Ok(match venue {
            Venue::BinanceSpot => Self::Binance(binance::router::Router::new(binance::router::Market::Spot, subs)?),
            Venue::BinanceFutures => {
                Self::Binance(binance::router::Router::new(binance::router::Market::Futures, subs)?)
            }
            Venue::Upbit | Venue::Bithumb => Self::Upbit(upbit::router::Router::new(venue, subs)?),
            Venue::Okx => Self::Okx(okx::router::Router::new(subs)?),
            Venue::BybitSpot | Venue::BybitLinear => Self::Bybit(bybit::router::Router::new(venue, subs)?),
            Venue::BitgetSpot | Venue::BitgetLinear => Self::Bitget(bitget::router::Router::new(venue, subs)?),
            Venue::GateSpot => Self::Gate(gate::router::Router::new(subs)?),
            Venue::KucoinSpot => Self::Kucoin(kucoin::router::Router::new(kucoin::router::Market::Spot, subs)?),
            Venue::KucoinFutures => {
                Self::Kucoin(kucoin::router::Router::new(kucoin::router::Market::Futures, subs)?)
            }
            Venue::Krx | Venue::Nxt | Venue::Smbs => return Err(RouterError::NotCrypto(venue)),
        })
    }

    /// The URL `venue` is dialled at when the conf names none.
    ///
    /// `None` for KuCoin, whose address comes from a ticket, and for a venue
    /// that is not crypto.
    pub const fn default_url(venue: Venue) -> Option<&'static str> {
        match venue {
            Venue::BinanceSpot => Some(binance::router::SPOT_URL),
            Venue::BinanceFutures => Some(binance::router::FUTURES_URL),
            Venue::Upbit | Venue::Bithumb => upbit::router::url(venue),
            Venue::Okx => Some(okx::router::URL),
            Venue::BybitSpot | Venue::BybitLinear => bybit::router::url(venue),
            Venue::BitgetSpot | Venue::BitgetLinear => Some(bitget::router::URL),
            Venue::GateSpot => Some(gate::router::URL),
            Venue::KucoinSpot | Venue::KucoinFutures | Venue::Krx | Venue::Nxt | Venue::Smbs => None,
        }
    }

    /// `true` if `venue` is one this crate routes.
    pub const fn is_crypto(venue: Venue) -> bool {
        !matches!(venue, Venue::Krx | Venue::Nxt | Venue::Smbs)
    }

    /// Points the venue's REST calls at `base` (`https://host[:port]`)
    /// instead of the venue's own. Only Gate and KuCoin make any; the others
    /// are unchanged.
    #[must_use]
    pub fn with_rest(self, base: &str) -> Self {
        match self {
            Self::Gate(r) => Self::Gate(r.with_rest(base)),
            Self::Kucoin(r) => Self::Kucoin(r.with_rest(base)),
            other => other,
        }
    }

    /// The subscriptions, in conf order.
    pub fn subscriptions_list(&self) -> &[Subscription] {
        match self {
            Self::Binance(r) => r.subscriptions_list(),
            Self::Upbit(r) => r.subscriptions_list(),
            Self::Okx(r) => r.subscriptions_list(),
            Self::Bybit(r) => r.subscriptions_list(),
            Self::Bitget(r) => r.subscriptions_list(),
            Self::Gate(r) => r.subscriptions_list(),
            Self::Kucoin(r) => r.subscriptions_list(),
        }
    }
}

impl Router for VenueRouter {
    fn venue(&self) -> Venue {
        match self {
            Self::Binance(r) => r.venue(),
            Self::Upbit(r) => r.venue(),
            Self::Okx(r) => r.venue(),
            Self::Bybit(r) => r.venue(),
            Self::Bitget(r) => r.venue(),
            Self::Gate(r) => r.venue(),
            Self::Kucoin(r) => r.venue(),
        }
    }

    fn route<S: RecordSink>(
        &mut self,
        msg: &[u8],
        recv_ns: UnixNano,
        sink: &mut S,
    ) -> Result<usize, CryptoError> {
        match self {
            Self::Binance(r) => r.route(msg, recv_ns, sink),
            Self::Upbit(r) => r.route(msg, recv_ns, sink),
            Self::Okx(r) => r.route(msg, recv_ns, sink),
            Self::Bybit(r) => r.route(msg, recv_ns, sink),
            Self::Bitget(r) => r.route(msg, recv_ns, sink),
            Self::Gate(r) => r.route(msg, recv_ns, sink),
            Self::Kucoin(r) => r.route(msg, recv_ns, sink),
        }
    }

    fn keepalive(&mut self, now: UnixNano, out: &mut [u8; MAX_KEEPALIVE_LEN]) -> Option<usize> {
        match self {
            Self::Binance(r) => r.keepalive(now, out),
            Self::Upbit(r) => r.keepalive(now, out),
            Self::Okx(r) => r.keepalive(now, out),
            Self::Bybit(r) => r.keepalive(now, out),
            Self::Bitget(r) => r.keepalive(now, out),
            Self::Gate(r) => r.keepalive(now, out),
            Self::Kucoin(r) => r.keepalive(now, out),
        }
    }

    fn subscriptions(&self) -> Vec<Vec<u8>> {
        match self {
            Self::Binance(r) => r.subscriptions(),
            Self::Upbit(r) => r.subscriptions(),
            Self::Okx(r) => r.subscriptions(),
            Self::Bybit(r) => r.subscriptions(),
            Self::Bitget(r) => r.subscriptions(),
            Self::Gate(r) => r.subscriptions(),
            Self::Kucoin(r) => r.subscriptions(),
        }
    }

    fn rest_books(&self) -> Vec<RestBook> {
        match self {
            Self::Binance(r) => r.rest_books(),
            Self::Upbit(r) => r.rest_books(),
            Self::Okx(r) => r.rest_books(),
            Self::Bybit(r) => r.rest_books(),
            Self::Bitget(r) => r.rest_books(),
            Self::Gate(r) => r.rest_books(),
            Self::Kucoin(r) => r.rest_books(),
        }
    }

    fn route_rest<S: RecordSink>(
        &mut self,
        instrument: usize,
        body: &[u8],
        recv_ns: UnixNano,
        sink: &mut S,
    ) -> Result<usize, CryptoError> {
        match self {
            Self::Binance(r) => r.route_rest(instrument, body, recv_ns, sink),
            Self::Upbit(r) => r.route_rest(instrument, body, recv_ns, sink),
            Self::Okx(r) => r.route_rest(instrument, body, recv_ns, sink),
            Self::Bybit(r) => r.route_rest(instrument, body, recv_ns, sink),
            Self::Bitget(r) => r.route_rest(instrument, body, recv_ns, sink),
            Self::Gate(r) => r.route_rest(instrument, body, recv_ns, sink),
            Self::Kucoin(r) => r.route_rest(instrument, body, recv_ns, sink),
        }
    }

    fn ticket(&self) -> Option<HttpRequest> {
        match self {
            Self::Binance(r) => r.ticket(),
            Self::Upbit(r) => r.ticket(),
            Self::Okx(r) => r.ticket(),
            Self::Bybit(r) => r.ticket(),
            Self::Bitget(r) => r.ticket(),
            Self::Gate(r) => r.ticket(),
            Self::Kucoin(r) => r.ticket(),
        }
    }

    fn endpoint_from_ticket(&mut self, body: &[u8]) -> Result<Endpoint, RouterError> {
        match self {
            Self::Binance(r) => r.endpoint_from_ticket(body),
            Self::Upbit(r) => r.endpoint_from_ticket(body),
            Self::Okx(r) => r.endpoint_from_ticket(body),
            Self::Bybit(r) => r.endpoint_from_ticket(body),
            Self::Bitget(r) => r.endpoint_from_ticket(body),
            Self::Gate(r) => r.endpoint_from_ticket(body),
            Self::Kucoin(r) => r.endpoint_from_ticket(body),
        }
    }
}
