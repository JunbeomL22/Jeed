//! Frames and instruments the Binance decoder tests share.
//!
//! The frames are the shapes Binance documents, byte for byte including the
//! trailing zeros it pads decimals with — those zeros are what
//! `to_i64_exact` has to accept, so a test that trimmed them would not be
//! testing the thing that matters.

use jeed_crypto::Instrument;
use jeed_wire::{Venue, WireRecord};

/// Spot BTCUSDT: two decimals of price, five of size.
pub fn spot_btcusdt() -> Instrument {
    Instrument::new(Venue::BinanceSpot, b"BTCUSDT", 2, 5).expect("valid instrument")
}

/// USD-M BTCUSDT: two decimals of price, three of size.
pub fn futures_btcusdt() -> Instrument {
    Instrument::new(Venue::BinanceFutures, b"BTCUSDT", 2, 3).expect("valid instrument")
}

/// A record with something already in it, so a test can prove a failed decode
/// left it alone.
pub fn dirty_record() -> WireRecord {
    let mut rec = WireRecord::zeroed();
    rec.header.producer_seq = 0xDEAD_BEEF;
    rec.header.kind = jeed_wire::WireKind::Heartbeat.as_u8();
    rec
}

/// Reception time used throughout, so a venue time is visibly different.
pub const RECV_NS: u64 = 1_755_088_800_000_000_000;

// ---------------------------------------------------------------------------
// Spot
// ---------------------------------------------------------------------------
//
// Every number is a multiple of the symbol's tickSize / stepSize, padded out
// to Binance's full display precision. That is not decoration: it is why
// reading at the configured scale is lossless, and a frame that broke it would
// mean the symbol's precision had changed under us.

/// `@bookTicker`. No `e`, no `E`, no `T` — spot's has no clock at all.
pub const SPOT_BOOK_TICKER: &[u8] = br#"{"u":400900217,"s":"BTCUSDT","b":"119250.01000000","B":"3.12100000","a":"119250.02000000","A":"4.06600000"}"#;

/// `@trade`, buyer is the maker, so the seller aggressed.
pub const SPOT_TRADE: &[u8] = br#"{"e":"trade","E":1755088771745,"s":"BTCUSDT","t":2723467893,"p":"4712.06000000","q":"3.84410000","T":1755088771744,"m":true,"M":true}"#;

/// `/api/v3/depth`, three bid levels and two ask levels.
pub const SPOT_SNAPSHOT: &[u8] = br#"{"lastUpdateId":1027024,"bids":[["4.00000000","431.00000000"],["3.99000000","12.50000000"],["3.98000000","1.00000000"]],"asks":[["4.01000000","12.00000000"],["4.02000000","5.00000000"]]}"#;

/// `@depth`, one change a side, the ask one a deletion.
pub const SPOT_DELTA: &[u8] = br#"{"e":"depthUpdate","E":1755088771745,"s":"BTCUSDT","U":157,"u":160,"b":[["119250.00000000","10.00000000"]],"a":[["119260.00000000","0"]]}"#;

// ---------------------------------------------------------------------------
// USD-M futures
// ---------------------------------------------------------------------------

/// `@bookTicker`, with both `E` and `T`.
pub const FUT_BOOK_TICKER: &[u8] = br#"{"e":"bookTicker","u":400900217,"E":1568014460893,"T":1568014460891,"s":"BTCUSDT","b":"119250.00","B":"31.210","a":"119250.10","A":"40.660"}"#;

/// `@aggTrade`, buyer is the taker, so the buyer aggressed.
pub const FUT_AGG_TRADE: &[u8] = br#"{"e":"aggTrade","E":1672515782136,"s":"BTCUSDT","a":164235345,"p":"7403.89","q":"100.000","f":100,"l":105,"T":1672515782136,"m":false}"#;

/// `/fapi/v1/depth` — the REST envelope.
pub const FUT_REST_SNAPSHOT: &[u8] = br#"{"lastUpdateId":1027024,"E":1606292218213,"T":1606292218208,"bids":[["4.00","431.000"]],"asks":[["4.01","12.000"]]}"#;

/// `@depth10@100ms` — a whole book in the `depthUpdate` envelope.
pub const FUT_PARTIAL_SNAPSHOT: &[u8] = br#"{"e":"depthUpdate","E":1571889248277,"T":1571889248276,"s":"BTCUSDT","U":390497796,"u":390497878,"pu":390497794,"b":[["7403.89","0.002"]],"a":[["7405.96","3.340"]]}"#;

/// `@depth`, with `pu`. The second bid is a deletion.
pub const FUT_DELTA: &[u8] = br#"{"e":"depthUpdate","E":1571889248277,"T":1571889248276,"s":"BTCUSDT","U":390497796,"u":390497878,"pu":390497794,"b":[["7403.89","0.002"],["7403.88","0"]],"a":[["7405.96","3.340"]]}"#;
