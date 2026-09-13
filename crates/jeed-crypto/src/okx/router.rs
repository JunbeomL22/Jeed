//! OKX's [`Router`]: the `arg` envelope, `op: subscribe`, and a bare `ping`.
//!
//! ```json
//! {"arg":{"channel":"books","instId":"BTC-USDT"},"action":"update","data":[…]}
//! ```
//!
//! ## Routing on `arg`
//!
//! Every data frame names its subscription in `arg` — `channel` and
//! `instId` — and the same two fields identify an acknowledgement
//! (`{"event":"subscribe","arg":…}`), which is why `event` is checked first:
//! a frame with one is conversation, not data. The decoders read the same
//! envelope again for their own checks; the second walk is a few dozen bytes
//! and buys a router that does not have to know what each decoder verifies.
//!
//! ## Channels
//!
//! | conf | OKX |
//! |---|---|
//! | `trade` | `trades` — batched, one record per print |
//! | `book` | `books5` — five levels, whole every time |
//! | `delta` | `books` — four hundred levels, a snapshot then diffs |
//!
//! `books50-l2-tbt` and `books-l2-tbt` need a VIP tier and are not offered
//! through the conf.
//!
//! ## `ping`, not a frame
//!
//! OKX closes a connection that has sent nothing for thirty seconds and asks
//! for the literal text `ping`; it answers `pong`, also literal text — a
//! message that is not JSON and routes as nothing. Sent every
//! [`PING_INTERVAL_NS`] regardless of traffic; that is simpler than tracking
//! our own silence and OKX does not mind.
//!
//! [`Router`]: crate::recv::Router

use crate::error::CryptoError;
use crate::json::{next_field, object_at, parse_string_bytes, skip_value};
use crate::okx;
use crate::recv::pipeline::MAX_KEEPALIVE_LEN;
use crate::recv::route::{self, Channel, ChannelSet, RouterError, Subscription, lookup, publish_batch};
use jeed_wire::{RecordSink, UnixNano, Venue};

/// The public WebSocket.
pub const URL: &str = "wss://ws.okx.com:8443/ws/v5/public";

/// How often to send `ping`. OKX's limit is thirty seconds of silence.
pub const PING_INTERVAL_NS: u64 = 25_000_000_000;

/// One OKX connection's instruments.
#[derive(Debug, Clone)]
pub struct Router {
    subs: Vec<Subscription>,
    next_ping_ns: UnixNano,
}

const SUPPORTED: ChannelSet = ChannelSet::EMPTY.with(Channel::Trade).with(Channel::Book).with(Channel::Delta);

impl Router {
    /// Builds a router. `bbo` and any `depth` are refused.
    pub fn new(subs: Vec<Subscription>) -> Result<Self, RouterError> {
        route::check(Venue::Okx, &subs, SUPPORTED, |_| false)?;
        Ok(Self { subs, next_ping_ns: 0 })
    }

    /// The subscriptions, in conf order.
    #[inline]
    pub fn subscriptions_list(&self) -> &[Subscription] {
        &self.subs
    }

    /// The `args` this router subscribes with, as `(channel, instId)`.
    pub fn args(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        for sub in &self.subs {
            let id = String::from_utf8_lossy(sub.instrument.symbol_bytes()).into_owned();
            for channel in sub.channels.iter() {
                let name = match channel {
                    Channel::Trade => "trades",
                    Channel::Book => "books5",
                    Channel::Delta => "books",
                    Channel::Bbo => unreachable!("refused by new"),
                };
                out.push((name, id.clone()));
            }
        }
        out
    }
}

/// `arg`'s two fields.
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
        Venue::Okx
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
                // An acknowledgement, an error, a `channel-conn-count` notice.
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

        // `pong`, or anything else that is not a data frame.
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
            b"trades" => {
                let mut prints = okx::trade::trades(msg)?;
                publish_batch(sink, |rec| prints.decode_next(inst, recv_ns, rec))
            }
            b"books" | b"books5" | b"books-l2-tbt" | b"books50-l2-tbt" => {
                sink.publish(|rec| okx::book::decode(inst, msg, recv_ns, rec))?;
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
        let mut msg = String::from(r#"{"op":"subscribe","args":["#);
        for (i, (channel, inst)) in self.args().iter().enumerate() {
            if i > 0 {
                msg.push(',');
            }
            msg.push_str(&format!(r#"{{"channel":"{channel}","instId":"{inst}"}}"#));
        }
        msg.push_str("]}");
        vec![msg.into_bytes()]
    }
}
