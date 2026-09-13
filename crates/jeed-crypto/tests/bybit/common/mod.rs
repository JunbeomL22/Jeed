//! Frames and instruments the Bybit decoder tests share.
//!
//! One JSON shape serves spot and linear, so the frames here are used with
//! both instruments — which is the claim the module makes and therefore the
//! claim worth testing.

use jeed_crypto::Instrument;
use jeed_wire::{Venue, WireRecord};

/// Bybit linear `BTCUSDT`: cent tick, three places of size.
pub fn linear_btcusdt() -> Instrument {
    Instrument::new(Venue::BybitLinear, b"BTCUSDT", 2, 3).expect("valid instrument")
}

/// Bybit spot `BTCUSDT` — the same string, a different instrument.
pub fn spot_btcusdt() -> Instrument {
    Instrument::new(Venue::BybitSpot, b"BTCUSDT", 2, 3).expect("valid instrument")
}

/// A record with something already in it, so a test can prove a failed decode
/// left it alone.
pub fn dirty_record() -> WireRecord {
    let mut rec = WireRecord::zeroed();
    rec.header.producer_seq = 0xDEAD_BEEF;
    rec.header.kind = jeed_wire::WireKind::Heartbeat.as_u8();
    rec
}

/// Reception time used throughout.
pub const RECV_NS: u64 = 1_672_304_500_000_000_000;

/// `orderbook.50.BTCUSDT`, the first frame after subscribing.
pub const ORDERBOOK_SNAPSHOT: &[u8] = br#"{"topic":"orderbook.50.BTCUSDT","type":"snapshot","ts":1672304484978,"data":{"s":"BTCUSDT","b":[["16493.50","0.006"],["16493.00","0.100"]],"a":[["16611.00","0.029"],["16612.00","0.213"]],"u":18521288,"seq":7961638724},"cts":1672304484976}"#;

/// `orderbook.50.BTCUSDT`, a diff. The second bid and the only ask are
/// deletions.
pub const ORDERBOOK_DELTA: &[u8] = br#"{"topic":"orderbook.50.BTCUSDT","type":"delta","ts":1672304485100,"data":{"s":"BTCUSDT","b":[["16493.20","30.028"],["16493.00","0"]],"a":[["16611.00","0"]],"u":18521289,"seq":7961638725},"cts":1672304485098}"#;

/// A `delta` frame with `u == 1` — Bybit restarted the orderbook service.
pub const ORDERBOOK_REBUILD: &[u8] = br#"{"topic":"orderbook.50.BTCUSDT","type":"delta","ts":1672304485200,"data":{"s":"BTCUSDT","b":[["16493.50","0.006"]],"a":[["16611.00","0.029"]],"u":1,"seq":7961638726},"cts":1672304485198}"#;

/// `publicTrade.BTCUSDT`, two prints in one frame.
pub const PUBLIC_TRADE: &[u8] = br#"{"topic":"publicTrade.BTCUSDT","type":"snapshot","ts":1672304486868,"data":[{"T":1672304486865,"s":"BTCUSDT","S":"Buy","v":"0.001","p":"16578.50","L":"PlusTick","i":"20f43950-d8dd-5b31-9112-a178eb6023af","BT":false},{"T":1672304486866,"s":"BTCUSDT","S":"Sell","v":"0.002","p":"16578.00","L":"MinusTick","i":"f7b1d1f6-1c1b-5a3c-9a7c-2b7d9c1e4a55","BT":false}]}"#;
