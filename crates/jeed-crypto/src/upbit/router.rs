//! Upbit's — and Bithumb's — [`Router`]: `type` + `code`, and the ticket
//! subscription.
//!
//! ```json
//! {"type":"trade","code":"KRW-BTC",…}
//! ```
//!
//! ## One router, two venues
//!
//! Bithumb's public WebSocket is Upbit's, message for message
//! ([`bithumb`](crate::bithumb)), and so is its subscription. The only
//! things that differ are the URL and the venue byte on the record, and the
//! latter comes from the [`Instrument`](crate::Instrument)s the router is
//! built with. So [`Router::new`] takes the venue and checks every
//! instrument agrees with it; the module is Upbit's because the protocol is.
//!
//! ## The envelope is the message
//!
//! There is no wrapper: `type` says the channel and `code` the market, at the
//! top level of the same object the decoder reads. The router reads those
//! two, finds the instrument, and hands the **whole** frame on. Both the
//! DEFAULT (`type`, `code`) and SIMPLE (`ty`, `cd`) spellings are matched —
//! the subscription below asks for DEFAULT, but a decoder that only worked
//! with the format it asked for would be one venue setting away from silent.
//!
//! ## Frames arrive binary
//!
//! Both venues send their JSON in **binary** frames. The loop routes binary
//! like text ([`receiver`](crate::recv::receiver)), so nothing is done here;
//! it is said so nobody reads [`Stats::binary`](crate::recv::Stats::binary)
//! climbing on this feed as a protocol change.
//!
//! ## Nothing to keep alive
//!
//! Upbit drops a connection idle for two minutes and answers a protocol
//! ping; the loop's silence ping covers it. No venue-level ping.
//!
//! [`Router`]: crate::recv::Router

use crate::error::CryptoError;
use crate::json::{next_field, parse_string_bytes, skip_value};
use crate::recv::route::{self, Channel, ChannelSet, RouterError, Subscription, lookup};
use crate::upbit;
use jeed_wire::{RecordSink, UnixNano, Venue};

/// Upbit's public WebSocket.
pub const UPBIT_URL: &str = "wss://api.upbit.com/websocket/v1";

/// Bithumb's public WebSocket (v2, Upbit-compatible).
pub const BITHUMB_URL: &str = "wss://ws-api.bithumb.com/websocket/v1";

/// The default URL for `venue`, or `None` if it is not one of the two.
pub const fn url(venue: Venue) -> Option<&'static str> {
    match venue {
        Venue::Upbit => Some(UPBIT_URL),
        Venue::Bithumb => Some(BITHUMB_URL),
        _ => None,
    }
}

/// One Upbit-protocol connection's instruments.
#[derive(Debug, Clone)]
pub struct Router {
    venue: Venue,
    subs: Vec<Subscription>,
}

const SUPPORTED: ChannelSet = ChannelSet::EMPTY.with(Channel::Trade).with(Channel::Book);

impl Router {
    /// Builds a router for [`Venue::Upbit`] or [`Venue::Bithumb`].
    ///
    /// `trade` and `book` are available; there is no diff stream and no
    /// depth choice, so `delta`, `bbo` and any `depth` are refused.
    pub fn new(venue: Venue, subs: Vec<Subscription>) -> Result<Self, RouterError> {
        if url(venue).is_none() {
            return Err(RouterError::NotCrypto(venue));
        }
        route::check(venue, &subs, SUPPORTED, |_| false)?;
        Ok(Self { venue, subs })
    }

    /// The subscriptions, in conf order.
    #[inline]
    pub fn subscriptions_list(&self) -> &[Subscription] {
        &self.subs
    }

    /// The `codes` array for `channel`: every symbol that wants it.
    fn codes(&self, channel: Channel) -> String {
        let mut out = String::new();
        for sub in self.subs.iter().filter(|s| s.channels.contains(channel)) {
            if !out.is_empty() {
                out.push(',');
            }
            out.push('"');
            out.push_str(&String::from_utf8_lossy(sub.instrument.symbol_bytes()));
            out.push('"');
        }
        out
    }
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
        let mut kind: Option<&[u8]> = None;
        let mut code: Option<&[u8]> = None;

        let mut pos = 0;
        while pos < msg.len() && (kind.is_none() || code.is_none()) {
            let (key, next) = next_field(msg, pos);
            if key.is_empty() {
                break;
            }
            match key {
                b"type" | b"ty" => {
                    let (s, p) = parse_string_bytes(msg, next);
                    kind = Some(s);
                    pos = p;
                }
                b"code" | b"cd" => {
                    let (s, p) = parse_string_bytes(msg, next);
                    code = Some(s);
                    pos = p;
                }
                _ => pos = skip_value(msg, next),
            }
        }

        // `{"status":"UP"}` answers a text PING; an error has `error`.
        let (Some(kind), Some(code)) = (kind, code) else {
            return Ok(0);
        };
        let Some((_, sub)) = lookup(&self.subs, code) else {
            return Ok(0);
        };
        let inst = &sub.instrument;
        match kind {
            b"trade" => sink.publish(|rec| upbit::trade::decode(inst, msg, recv_ns, rec))?,
            b"orderbook" => sink.publish(|rec| upbit::snapshot::decode(inst, msg, recv_ns, rec))?,
            _ => return Ok(0),
        }
        Ok(1)
    }

    fn subscriptions(&self) -> Vec<Vec<u8>> {
        // One array: a ticket, then one object per channel with every code
        // that wants it. DEFAULT key spelling — no `format` field.
        let mut msg = String::from(r#"[{"ticket":"jeed"}"#);
        for (channel, name) in [(Channel::Trade, "trade"), (Channel::Book, "orderbook")] {
            let codes = self.codes(channel);
            if !codes.is_empty() {
                msg.push_str(&format!(r#",{{"type":"{name}","codes":[{codes}]}}"#));
            }
        }
        msg.push(']');
        vec![msg.into_bytes()]
    }
}
