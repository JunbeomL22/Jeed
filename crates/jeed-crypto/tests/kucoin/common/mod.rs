//! Frames and instruments the KuCoin decoder tests share.

use jeed_crypto::Instrument;
use jeed_wire::{Venue, WireRecord};

/// KuCoin spot `BTC-USDT`: one decimal place of price, eight of size.
pub fn spot_btc_usdt() -> Instrument {
    Instrument::new(Venue::KucoinSpot, b"BTC-USDT", 1, 8).expect("valid instrument")
}

/// KuCoin futures `XBTUSDTM`: one decimal place of price, and **no** size
/// decimals — a futures size is a whole number of contracts.
pub fn futures_xbtusdtm() -> Instrument {
    Instrument::new(Venue::KucoinFutures, b"XBTUSDTM", 1, 0).expect("valid instrument")
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
pub const RECV_NS: u64 = 1_545_913_900_000_000_000;

/// `/market/level2` — a spot diff, both sides, three-element levels.
pub const SPOT_LEVEL2: &[u8] = br#"{"type":"message","topic":"/market/level2:BTC-USDT","subject":"trade.l2update","data":{"sequenceStart":1545896669105,"sequenceEnd":1545896669107,"symbol":"BTC-USDT","changes":{"asks":[["3536.1","0.10000000","1545896669105"],["3536.5","0.00000000","1545896669106"]],"bids":[["3535.5","0.03000000","1545896669107"]]}}}"#;

/// `/market/match` — a spot print, with a nanosecond `time`.
pub const SPOT_MATCH: &[u8] = br#"{"type":"message","topic":"/market/match:BTC-USDT","subject":"trade.l3match","data":{"sequence":"1545896669145","type":"match","symbol":"BTC-USDT","side":"buy","price":"3535.50000000","size":"0.01022222","tradeId":"5c24c5da03aa673885cd67aa","takerOrderId":"5c24c5d903aa6772d55b371e","makerOrderId":"5c2187d003aa677bd09f5c6c","time":"1545913818099321004"}}"#;

/// `/api/v3/market/orderbook/level2` — the spot REST body, with a
/// **millisecond** `time`.
pub const SPOT_REST: &[u8] = br#"{"code":"200000","data":{"time":1602997267139,"sequence":"1602997267139","bids":[["3535.5","0.03000000"],["3535.0","1.00000000"]],"asks":[["3536.1","0.10000000"]]}}"#;

/// `/contractMarket/level2` — a futures diff: one level, in a string.
pub const FUTURES_LEVEL2: &[u8] = br#"{"topic":"/contractMarket/level2:XBTUSDTM","type":"message","subject":"level2","data":{"sequence":1709400450243,"change":"90631.2,sell,2","timestamp":1731897467182}}"#;

/// `/contractMarket/execution` — a futures print: bare integer size, bare
/// nanosecond `ts`.
pub const FUTURES_EXECUTION: &[u8] = br#"{"topic":"/contractMarket/execution:XBTUSDTM","type":"message","subject":"match","data":{"symbol":"XBTUSDTM","sequence":1743169228057,"side":"buy","size":5,"price":"72137.3","takerOrderId":"166754573174730752","makerOrderId":"166754559966867456","tradeId":"1743169228057","ts":1712570590399000000}}"#;

/// `/api/v1/level2/snapshot` — the futures REST body, quoted prices beside
/// bare sizes.
pub const FUTURES_REST: &[u8] = br#"{"code":"200000","data":{"symbol":"XBTUSDTM","sequence":1709400450243,"asks":[["90631.2",2],["90632.0",7]],"bids":[["90630.8",5]]}}"#;
