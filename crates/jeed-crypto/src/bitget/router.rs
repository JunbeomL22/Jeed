//! Bitget's [`Router`]: the `arg` envelope with `instType`, and a bare `ping`.
//!
//! ```json
//! {"action":"snapshot","arg":{"instType":"SPOT","channel":"books","instId":"BTCUSDT"},"data":[…]}
//! ```
//!
//! OKX's envelope with one more field, and the router is OKX's with one
//! more field: `instType` goes into the subscription (`SPOT` or
//! `USDT-FUTURES`, from the venue) and is not read on the way back — the
//! caller knew which market it subscribed on ([`bitget`]).
//!
//! ## Channels
//!
//! | conf | Bitget |
//! |---|---|
//! | `trade` | `trade` — batched |
//! | `book` | `books` (whole then diffs), or `books1` / `books5` / `books15` with `depth` |
//!
//! ## `ping` every thirty seconds
//!
//! Literal text, answered with literal `pong`, which is not JSON and routes
//! as nothing.
//!
//! [`Router`]: crate::recv::Router

use crate::bitget;
use crate::error::CryptoError;
use crate::json::{next_field, object_at, parse_string_bytes, skip_value};
use crate::recv::pipeline::MAX_KEEPALIVE_LEN;
use crate::recv::route::{self, Channel, ChannelSet, RouterError, Subscription, lookup, publish_batch};
use jeed_wire::{RecordSink, UnixNano, Venue};

/// The public WebSocket, both markets.
pub const URL: &str = "wss://ws.bitget.com/v2/ws/public";

/// How often to send `ping`.
pub const PING_INTERVAL_NS: u64 = 30_000_000_000;

/// `arg.instType` for `venue`, or `None` if it is not a Bitget venue.
pub const fn inst_type(venue: Venue) -> Option<&'static str> {
    match venue {
        Venue::BitgetSpot => Some("SPOT"),
        Venue::BitgetLinear => Some("USDT-FUTURES"),
        _ => None,
    }
}

/// One Bitget connection's instruments.
#[derive(Debug, Clone)]
pub struct Router {
    venue: Venue,
    subs: Vec<Subscription>,
    next_ping_ns: UnixNano,
}

const SUPPORTED: ChannelSet = ChannelSet::EMPTY.with(Channel::Trade).with(Channel::Book);

impl Router {
    /// Builds a router for [`Venue::BitgetSpot`] or [`Venue::BitgetLinear`].
    pub fn new(venue: Venue, subs: Vec<Subscription>) -> Result<Self, RouterError> {
        if inst_type(venue).is_none() {
            return Err(RouterError::NotCrypto(venue));
        }
        route::check(venue, &subs, SUPPORTED, |d| matches!(d, 1 | 5 | 15))?;
        Ok(Self { venue, subs, next_ping_ns: 0 })
    }

    /// The subscriptions, in conf order.
    #[inline]
    pub fn subscriptions_list(&self) -> &[Subscription] {
        &self.subs
    }

    /// The `args` this router subscribes with, as `(channel, instId)`.
    pub fn args(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for sub in &self.subs {
            let id = String::from_utf8_lossy(sub.instrument.symbol_bytes()).into_owned();
            if sub.channels.contains(Channel::Trade) {
                out.push(("trade".to_owned(), id.clone()));
            }
            if sub.channels.contains(Channel::Book) {
                let channel = match sub.depth {
                    None => "books".to_owned(),
                    Some(d) => format!("books{d}"),
                };
                out.push((channel, id));
            }
        }
        out
    }
}

/// `arg`'s `channel` and `instId`.
fn arg_fields(arg: &[u8]) -> (Option<&[u8]>, Option<&[u8]>) {
    let (mut channel, mut inst) = (None, None);
    let mut pos = 0;
    while pos < arg.len() {
        let (key, next) = next_field(arg, pos);
        if key.is_empty() {
            break;
        }
        match key {
            b"channel" => {
                let (s, p) = parse_string_bytes(arg, next);
                channel = Some(s);
                pos = p;
            }
            b"instId" => {
                let (s, p) = parse_string_bytes(arg, next);
                inst = Some(s);
                pos = p;
            }
            _ => pos = skip_value(arg, next),
        }
    }
    (channel, inst)
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
        let mut arg: Option<&[u8]> = None;
        let mut has_data = false;

        let mut pos = 0;
        while pos < msg.len() {
            let (key, next) = next_field(msg, pos);
            if key.is_empty() {
                break;
            }
            match key {
                b"event" => return Ok(0),
                b"arg" => match object_at(msg, next) {
                    Some((inner, p)) => {
                        arg = Some(inner);
                        pos = p;
                    }
                    None => pos = skip_value(msg, next),
                },
                b"data" => {
                    has_data = true;
                    pos = skip_value(msg, next);
                }
                _ => pos = skip_value(msg, next),
            }
        }

        let Some(arg) = arg else {
            return Ok(0);
        };
        if !has_data {
            return Ok(0);
        }
        let (Some(channel), Some(inst_id)) = arg_fields(arg) else {
            return Ok(0);
        };
        let Some((_, sub)) = lookup(&self.subs, inst_id) else {
            return Ok(0);
        };
        let inst = &sub.instrument;

        match channel {
            b"trade" => {
                let mut prints = bitget::trade::trades(inst, msg)?;
                publish_batch(sink, |rec| prints.decode_next(inst, recv_ns, rec))
            }
            c if c.starts_with(b"books") => {
                sink.publish(|rec| bitget::book::decode(inst, msg, recv_ns, rec))?;
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
        out[..4].copy_from_slice(b"ping");
        Some(4)
    }

    fn subscriptions(&self) -> Vec<Vec<u8>> {
        let inst_type = inst_type(self.venue).expect("checked by new");
        let mut msg = String::from(r#"{"op":"subscribe","args":["#);
        for (i, (channel, inst)) in self.args().iter().enumerate() {
            if i > 0 {
                msg.push(',');
            }
            msg.push_str(&format!(
                r#"{{"instType":"{inst_type}","channel":"{channel}","instId":"{inst}"}}"#
            ));
        }
        msg.push_str("]}");
        vec![msg.into_bytes()]
    }
}
