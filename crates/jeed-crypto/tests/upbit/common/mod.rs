//! Frames and instruments the Upbit decoder tests share.
//!
//! Every frame appears twice, once in DEFAULT spelling and once in SIMPLE,
//! because a subscription picks one and then gets it for the whole connection
//! — a decoder that only handled one of them would be wrong half the time and
//! would look right in a test suite that only carried one of them.
//!
//! Prices and sizes are unquoted, as Upbit sends them, and are multiples of
//! the market's tick and lot padded out to full precision.

use jeed_crypto::Instrument;
use jeed_wire::{Venue, WireRecord};

/// Upbit KRW-BTC: KRW prices are whole won, sizes go to eight places.
pub fn krw_btc() -> Instrument {
    Instrument::new(Venue::Upbit, b"KRW-BTC", 0, 8).expect("valid instrument")
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
pub const RECV_NS: u64 = 1_704_067_300_000_000_000;

/// `orderbook`, DEFAULT keys, two ranks.
pub const ORDERBOOK: &[u8] = br#"{"type":"orderbook","code":"KRW-BTC","timestamp":1704067200000,"total_ask_size":1.23456789,"total_bid_size":2.34567891,"orderbook_units":[{"ask_price":152430000.0,"bid_price":152400000.0,"ask_size":0.17000000,"bid_size":0.23000000},{"ask_price":152440000.0,"bid_price":152390000.0,"ask_size":0.05000000,"bid_size":0.11000000}],"stream_type":"REALTIME","level":0}"#;

/// The same book in SIMPLE spelling.
pub const ORDERBOOK_SIMPLE: &[u8] = br#"{"ty":"orderbook","cd":"KRW-BTC","tms":1704067200000,"tas":1.23456789,"tbs":2.34567891,"obu":[{"ap":152430000.0,"bp":152400000.0,"as":0.17,"bs":0.23},{"ap":152440000.0,"bp":152390000.0,"as":0.05,"bs":0.11}],"st":"REALTIME","lv":0}"#;

/// `trade`, DEFAULT keys. `BID` means the buyer took liquidity.
pub const TRADE: &[u8] = br#"{"type":"trade","code":"KRW-BTC","timestamp":1704067200123,"trade_date":"2024-01-01","trade_time":"00:00:00","trade_timestamp":1704067200000,"trade_price":152430000.0,"trade_volume":0.00084280,"ask_bid":"BID","prev_closing_price":151000000.0,"change":"RISE","change_price":1430000.0,"sequential_id":1704067200000000,"stream_type":"REALTIME"}"#;

/// The same print in SIMPLE spelling, on the other side.
pub const TRADE_SIMPLE: &[u8] = br#"{"ty":"trade","cd":"KRW-BTC","tms":1704067200123,"tdt":"2024-01-01","ttm":"00:00:00","ttms":1704067200000,"tp":152430000.0,"tv":0.00084280,"ab":"ASK","pcp":151000000.0,"c":"FALL","cp":1430000.0,"sid":1704067200000000,"st":"REALTIME"}"#;
