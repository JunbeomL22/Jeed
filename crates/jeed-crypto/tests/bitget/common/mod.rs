//! Frames and instruments the Bitget decoder tests share.
//!
//! The frames keep Bitget's own shape: an `arg` envelope that carries the only
//! copy of the symbol, a `data` array of one object for the book, and quoted
//! numbers throughout.

use jeed_crypto::Instrument;
use jeed_wire::{Venue, WireRecord};

/// Bitget `BTCUSDT` USDT-margined perpetual: a tenth-dollar tick, three
/// places of size.
pub fn linear_btcusdt() -> Instrument {
    Instrument::new(Venue::BitgetLinear, b"BTCUSDT", 1, 3).expect("valid instrument")
}

/// The spot pair of the same name — a different book on a different venue
/// byte, decoded by the same functions.
pub fn spot_btcusdt() -> Instrument {
    Instrument::new(Venue::BitgetSpot, b"BTCUSDT", 1, 3).expect("valid instrument")
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
pub const RECV_NS: u64 = 1_695_716_100_000_000_000;

/// `books`, the first frame after subscribing.
pub const BOOKS_SNAPSHOT: &[u8] = br#"{"action":"snapshot","arg":{"instType":"USDT-FUTURES","channel":"books","instId":"BTCUSDT"},"data":[{"asks":[["27000.5","8.760"],["27003.0","0.500"]],"bids":[["27000.0","2.710"],["26999.5","1.000"]],"checksum":-855196043,"seq":"456","pseq":"455","ts":"1695716059616"}]}"#;

/// `books`, a diff. `asks` arrives first here too, so the shared level array
/// has to be ordered after both sides are in.
pub const BOOKS_UPDATE: &[u8] = br#"{"action":"update","arg":{"instType":"USDT-FUTURES","channel":"books","instId":"BTCUSDT"},"data":[{"asks":[["27003.0","0.000"]],"bids":[["27000.0","3.100"],["26998.0","0.250"]],"checksum":123456,"seq":"457","pseq":"456","ts":"1695716059716"}]}"#;

/// `books5` — a shallow book, still typed `snapshot` and still whole.
pub const BOOKS5: &[u8] = br#"{"action":"snapshot","arg":{"instType":"SPOT","channel":"books5","instId":"BTCUSDT"},"data":[{"asks":[["27000.5","8.760"]],"bids":[["27000.0","2.710"]],"checksum":0,"seq":"99","ts":"1695716059616"}]}"#;

/// `trade`, two prints in one frame. The entries carry no symbol at all.
pub const TRADES: &[u8] = br#"{"action":"update","arg":{"instType":"USDT-FUTURES","channel":"trade","instId":"BTCUSDT"},"data":[{"ts":"1695716760565","price":"27000.5","size":"0.001","side":"buy","tradeId":"1111111111"},{"ts":"1695716760566","price":"27000.0","size":"0.002","side":"sell","tradeId":"1111111112"}]}"#;

/// `/api/v2/mix/market/merge-depth` — the REST body, where `data` is an
/// object and there is no sequence number anywhere.
pub const REST_DEPTH: &[u8] = br#"{"code":"00000","msg":"success","requestTime":1706000000000,"data":{"asks":[["27001.0","1.200"],["27002.0","0.500"]],"bids":[["27000.0","2.300"]],"ts":"1706000000000"}}"#;
