//! Bybit V5 market data.
//!
//! ```text
//! orderbook.{depth}.{symbol}  → book   Quote or SnapshotDelta
//! publicTrade.{symbol}        → trade  WireKind::Trade, batched
//! ```
//!
//! | | spot | linear |
//! |---|---|---|
//! | venue | [`Venue::BybitSpot`](jeed_wire::Venue::BybitSpot) | [`Venue::BybitLinear`](jeed_wire::Venue::BybitLinear) |
//! | 호스트 | `wss://stream.bybit.com/v5/public/spot` | `wss://stream.bybit.com/v5/public/linear` |
//! | 전문 | 같다 | 같다 |
//!
//! ## Two venue bytes, one set of decoders
//!
//! Bybit spells the linear perpetual `BTCUSDT`, exactly as it spells the spot
//! pair, and only the endpoint tells them apart — so they must not share a
//! venue byte, or `(venue, symbol)` would merge two books
//! (`documents/feed_handler.md` §6). That is the same argument the Binance
//! module makes.
//!
//! It does **not** follow that they need a module each. Binance's markets got
//! [`spot`](crate::binance::spot) and [`futures`](crate::binance::futures)
//! because the messages differ — different trade streams with different names
//! and fields, different book envelopes. Bybit's do not: one JSON shape serves
//! both categories, and the endpoint is the whole of the difference. Splitting
//! here would put two copies of the same walk in two files for them to drift
//! apart in, which is what `CLAUDE.md` says not to do — the market is a
//! property of the [`Instrument`](crate::Instrument), not of the module.
//!
//! Inverse contracts, which are also `BTCUSD`-shaped and also share the format,
//! take a third venue byte and no more code.
//!
//! ## The envelope
//!
//! ```json
//! {"topic":"orderbook.50.BTCUSDT","type":"snapshot","ts":1672304484978,
//!  "data":{…},"cts":1672304484976}
//! ```
//!
//! `topic`, `type` and `ts` share a first byte, so these decoders read whole
//! keys ([`next_field`](crate::json::next_field)) rather than Binance's first
//! byte.
//!
//! ## `seq` is not carried
//!
//! Every book message has both `u`, which chains the orderbook stream, and
//! `seq`, which orders events across *different* streams for the same symbol
//! — so that a `orderbook.1` and an `orderbook.50` subscription can be told
//! which of their messages came first. That is a cross-subscription fact, the
//! wire record has no field for it, and inventing one would carry a number
//! only a consumer subscribed to two depths of the same symbol could use.
//! `u` is what chains a book, and `u` is what is carried.

pub mod book;
#[cfg(feature = "recv")]
pub mod router;
pub mod trade;

/// Bit positions in a Bybit decoder's [`crate::mask`] word.
pub(crate) mod seen {
    /// `b` — bid side of a book message.
    pub const BIDS: u8 = 1 << 0;
    /// `a` — ask side of a book message.
    pub const ASKS: u8 = 1 << 1;
    /// `u` — orderbook update id.
    pub const UPDATE_ID: u8 = 1 << 2;
    /// `p` — trade price.
    pub const PRICE: u8 = 1 << 3;
    /// `v` — trade quantity.
    pub const QTY: u8 = 1 << 4;
}
