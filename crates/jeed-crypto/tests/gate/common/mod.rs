//! Frames and instruments the Gate decoder tests share.

use jeed_crypto::Instrument;
use jeed_wire::{Venue, WireRecord};

/// Gate `BTC_USDT` spot: two decimal places of price, four of size.
pub fn btc_usdt() -> Instrument {
    Instrument::new(Venue::GateSpot, b"BTC_USDT", 2, 4).expect("valid instrument")
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
pub const RECV_NS: u64 = 1_606_294_800_000_000_000;

/// `spot.order_book_update`, a diff. Bids arrive first here, unlike OKX.
pub const ORDER_BOOK_UPDATE: &[u8] = br#"{"time":1606294781,"time_ms":1606294781236,"channel":"spot.order_book_update","event":"update","result":{"t":1606294781123,"e":"depthUpdate","E":1606294781,"s":"BTC_USDT","U":48776301,"u":48776306,"b":[["19137.74","0.0001"],["19088.37","0"]],"a":[["19137.75","0.6135"]]}}"#;

/// `spot.trades`, one print — which is all Gate ever sends in a frame.
pub const TRADE: &[u8] = br#"{"time":1606292218,"time_ms":1606292218231,"channel":"spot.trades","event":"update","result":{"id":309143071,"create_time":1606292218,"create_time_ms":"1606292218213.4578","side":"sell","currency_pair":"BTC_USDT","amount":"16.4700","price":"19137.75"}}"#;

/// `/api/v4/spot/order_book?with_id=true` — the REST body.
pub const REST_ORDER_BOOK: &[u8] = br#"{"id":48776300,"current":1606295412123,"update":1606295412100,"asks":[["19137.75","0.6135"],["19138.00","1.0000"]],"bids":[["19137.74","0.0001"]]}"#;

/// A subscribe answer. Same socket, same envelope, no book in it.
pub const SUBSCRIBE_ACK: &[u8] = br#"{"time":1606294781,"time_ms":1606294781236,"channel":"spot.order_book_update","event":"subscribe","result":{"status":"success"}}"#;
