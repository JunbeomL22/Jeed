//! Frames and instruments the OKX decoder tests share.
//!
//! Levels are four-element arrays — `[price, size, "0", order count]` — where
//! the third is a deprecated field OKX still sends and the fourth is a count
//! this crate deliberately drops. Keeping them in the frames is the point: a
//! two-element test frame would not prove the reader steps over them.

use jeed_crypto::Instrument;
use jeed_wire::{Venue, WireRecord};

/// OKX `BTC-USDT` spot: a tenth-dollar tick, eight places of size.
pub fn btc_usdt() -> Instrument {
    Instrument::new(Venue::Okx, b"BTC-USDT", 1, 8).expect("valid instrument")
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
pub const RECV_NS: u64 = 1_706_000_100_000_000_000;

/// `books`, the first frame after subscribing.
pub const BOOKS_SNAPSHOT: &[u8] = br#"{"arg":{"channel":"books","instId":"BTC-USDT"},"action":"snapshot","data":[{"asks":[["64000.5","0.01000000","0","1"],["64001.0","0.02000000","0","2"]],"bids":[["63999.5","0.02000000","0","1"],["63999.0","0.10000000","0","3"]],"ts":"1706000000000","checksum":-855196043,"seqId":1234567890,"prevSeqId":1234567889}]}"#;

/// `books`, a diff. `asks` arrives first, which is OKX's habit and the reason
/// the shared level array has to be ordered after the fact.
pub const BOOKS_UPDATE: &[u8] = br#"{"arg":{"channel":"books","instId":"BTC-USDT"},"action":"update","data":[{"asks":[["64002.0","0","0","0"]],"bids":[["63999.5","0.05000000","0","2"],["63998.0","0.30000000","0","4"]],"ts":"1706000000100","checksum":123456,"seqId":1234567891,"prevSeqId":1234567890}]}"#;

/// `books5` — a whole book with no `action` key at all.
pub const BOOKS5: &[u8] = br#"{"arg":{"channel":"books5","instId":"BTC-USDT"},"data":[{"asks":[["64000.5","0.01000000","0","1"]],"bids":[["63999.5","0.02000000","0","1"]],"instId":"BTC-USDT","ts":"1706000000000","seqId":1234567890}]}"#;

/// `trades`, two prints in one frame.
pub const TRADES: &[u8] = br#"{"arg":{"channel":"trades","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","tradeId":"130639474","px":"64000.5","sz":"0.12000000","side":"buy","ts":"1706000000000","count":"3"},{"instId":"BTC-USDT","tradeId":"130639475","px":"64000.4","sz":"0.03000000","side":"sell","ts":"1706000000001","count":"1"}]}"#;
