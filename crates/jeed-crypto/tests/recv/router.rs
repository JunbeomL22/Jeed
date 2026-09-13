//! A stand-in venue router, and the tests that pin its shape.
//!
//! `jeed_crypto::recv` ships no router yet — which stream a message belongs
//! to is the venue layer, and that arrives with the binary — so the pipeline
//! cannot be tested without one. This one is the smallest thing that has the
//! two behaviours the loop has to cope with:
//!
//! - a message that is **for** it and decodes (a Binance spot trade), or
//!   does not;
//! - a message that is **not** for it and is fine — a subscription
//!   acknowledgement — which is zero records, not an error.
//!
//! And it can want a venue keepalive on a schedule, so the loop's
//! keepalive path has something to send.

use crate::frames::{RECV_NS, SPOT_TRADE, spot_btcusdt};
use crate::sink::Collect;
use jeed_crypto::recv::{MAX_KEEPALIVE_LEN, Router};
use jeed_crypto::{CryptoError, Instrument, binance};
use jeed_wire::{RecordSink, UnixNano, Venue, WireKind};

/// What the fake venue sends as its own ping.
pub const KEEPALIVE: &[u8] = br#"{"op":"ping"}"#;

/// A venue that carries one Binance spot instrument and acknowledges
/// subscriptions with `{"result":null,…}`.
#[derive(Debug)]
pub struct Fake {
    inst: Instrument,

    /// Send [`KEEPALIVE`] this often. Zero never sends one.
    pub keepalive_every_ns: u64,

    next_keepalive_ns: UnixNano,
}

impl Fake {
    /// BTCUSDT on Binance spot, no keepalive.
    pub fn new() -> Self {
        Self { inst: spot_btcusdt(), keepalive_every_ns: 0, next_keepalive_ns: 0 }
    }

    /// The same, sending [`KEEPALIVE`] every `every_ns`.
    pub fn with_keepalive(every_ns: u64) -> Self {
        Self { keepalive_every_ns: every_ns, ..Self::new() }
    }
}

impl Router for Fake {
    fn venue(&self) -> Venue {
        Venue::BinanceSpot
    }

    fn route<S: RecordSink>(
        &mut self,
        msg: &[u8],
        recv_ns: UnixNano,
        sink: &mut S,
    ) -> Result<usize, CryptoError> {
        if msg.starts_with(br#"{"result""#) {
            return Ok(0);
        }
        sink.publish(|rec| binance::spot::trade::decode(&self.inst, msg, recv_ns, rec))?;
        Ok(1)
    }

    fn keepalive(&mut self, now: UnixNano, out: &mut [u8; MAX_KEEPALIVE_LEN]) -> Option<usize> {
        if self.keepalive_every_ns == 0 || now < self.next_keepalive_ns {
            return None;
        }
        self.next_keepalive_ns = now.saturating_add(self.keepalive_every_ns);
        out[..KEEPALIVE.len()].copy_from_slice(KEEPALIVE);
        Some(KEEPALIVE.len())
    }
}

#[test]
fn a_trade_is_one_record() {
    let mut sink = Collect::new();
    let n = Fake::new().route(SPOT_TRADE, RECV_NS, &mut sink).expect("decodes");

    assert_eq!(n, 1);
    assert_eq!(sink.len(), 1);
    assert_eq!(sink.last().header.kind, WireKind::Trade.as_u8());
}

#[test]
fn an_acknowledgement_is_zero_records_and_no_error() {
    let mut sink = Collect::new();
    let n = Fake::new().route(br#"{"result":null,"id":1}"#, RECV_NS, &mut sink).expect("fine");

    assert_eq!(n, 0);
    assert_eq!(sink.len(), 0);
}

#[test]
fn junk_is_refused_and_publishes_nothing() {
    let mut sink = Collect::new();
    let out = Fake::new().route(br#"{"e":"trade","s":"ETHUSDT"}"#, RECV_NS, &mut sink);

    assert!(out.is_err());
    assert_eq!(sink.len(), 0);
}

#[test]
fn the_keepalive_fires_on_schedule() {
    let mut r = Fake::with_keepalive(100);
    let mut out = [0u8; MAX_KEEPALIVE_LEN];

    let n = r.keepalive(1_000, &mut out).expect("due at once");
    assert_eq!(&out[..n], KEEPALIVE);
    assert_eq!(r.keepalive(1_050, &mut out), None, "not yet");
    assert!(r.keepalive(1_100, &mut out).is_some(), "due again");
}

#[test]
fn zero_never_sends_a_keepalive() {
    let mut out = [0u8; MAX_KEEPALIVE_LEN];
    assert_eq!(Fake::new().keepalive(u64::MAX, &mut out), None);
}
