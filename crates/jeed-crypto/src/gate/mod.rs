//! Gate.io V4 spot market data (`wss://api.gateio.ws/ws/v4/`).
//!
//! ```text
//! spot.order_book_update  → delta     WireKind::SnapshotDelta
//! spot.trades             → trade     WireKind::Trade, one per frame
//! /api/v4/spot/order_book → snapshot  Quote (REST body)
//! ```
//!
//! ## Binance's chain in Gate's envelope
//!
//! The book channel numbers its diffs `U`…`u` exactly as Binance spot does —
//! a range per message, no named predecessor — and the consumer chains them
//! the same way. What differs is everything around the numbers: the payload
//! sits in a `result` object, the sides are `b` and `a`, and the frame carries
//! two clocks of its own before `result` has its third.
//!
//! ```json
//! {"time":1606294781,"time_ms":1606294781236,
//!  "channel":"spot.order_book_update","event":"update",
//!  "result":{"t":1606294781123,"e":"depthUpdate","E":1606294781,"s":"BTC_USDT",
//!            "U":48776301,"u":48776306,
//!            "b":[["19137.74","0.0001"]],"a":[["19137.75","0.6135"]]}}
//! ```
//!
//! ## There is no book snapshot on the socket
//!
//! Gate pushes diffs and nothing else; the whole book comes from
//! `/api/v4/spot/order_book`, which is why [`snapshot`] decodes a REST body
//! rather than a frame. The seam between the two is the consumer's: it holds
//! the book, so it is the only side that can hold a diff back until the
//! snapshot it belongs after has arrived (`CLAUDE.md`).
//!
//! ## `event` is not read
//!
//! Subscribe and unsubscribe answers ride the same socket with
//! `"event":"subscribe"` and a `result` of `{"status":"success"}`. They are
//! not book messages, and they decode to [`CryptoError::Missing`] because they
//! have no `u` in them — which is the same conclusion an `event` check would
//! reach, from the fields that would actually be used rather than from a label.
//!
//! ## One venue byte, and a market in its name
//!
//! [`GateSpot`](jeed_wire::Venue::GateSpot) is spot alone. Gate's perpetuals
//! spell the contract `BTC_USDT` too, so the second market cannot share this
//! byte when it is ported — the Binance and Bybit case, decided in advance
//! this time.
//!
//! [`CryptoError::Missing`]: crate::CryptoError::Missing

pub mod delta;
pub mod snapshot;
pub mod trade;

/// Bit positions in a Gate decoder's [`crate::mask`] word.
pub(crate) mod seen {
    /// `b` — bids.
    pub const BIDS: u8 = 1 << 0;
    /// `a` — asks.
    pub const ASKS: u8 = 1 << 1;
    /// `u` — the last update id this message covers.
    pub const FINAL: u8 = 1 << 2;
    /// `U` — the first.
    pub const FIRST: u8 = 1 << 3;
    /// `price`.
    pub const PRICE: u8 = 1 << 4;
    /// `amount`.
    pub const QTY: u8 = 1 << 5;
}
