//! OKX V5 market data (`wss://ws.okx.com:8443/ws/v5/public`).
//!
//! ```text
//! books, books-l2-tbt, books50-l2-tbt  → book   Quote or SnapshotDelta
//! books5                               → book   Quote (no action field)
//! trades                               → trade  WireKind::Trade, batched
//! ```
//!
//! ## One venue byte, unlike Binance and Bybit
//!
//! [`Venue::Okx`](jeed_wire::Venue::Okx) covers spot, perpetuals, dated
//! futures and options together, because OKX's `instId` already separates
//! them: `BTC-USDT` is the spot pair, `BTC-USDT-SWAP` the perpetual,
//! `BTC-USDT-240329` a dated future and `BTC-USD-240329-70000-C` an option.
//! `(venue, symbol)` is therefore already unique, and there is nothing for a
//! second venue byte to fix — where Binance and Bybit both spell a spot pair
//! and a perpetual `BTCUSDT` and need one.
//!
//! Options are also why the wire symbol is twenty-four bytes:
//! `BTC-USD-240329-70000-C` is twenty-two of them.
//!
//! ## Everything arrives in an envelope
//!
//! ```json
//! {"arg":{"channel":"books","instId":"BTC-USDT"},"action":"update","data":[…]}
//! ```
//!
//! `arg` names the subscription and `data` is always an array, even when it
//! holds one object. Both decoders take the envelope apart with
//! [`object_at`](crate::json::object_at) / [`objects_at`](crate::json::objects_at)
//! and walk the inner bytes as a message of their own.
//!
//! ## `checksum` is not carried
//!
//! Every `books` message ships a CRC32 over the top twenty-five levels of the
//! book *as the consumer should now hold it*. It is genuinely useful and it is
//! deliberately dropped, for two reasons that point the same way: the feed
//! handler holds no book, so it cannot check the checksum itself; and the
//! rule for computing it — twenty-five levels, `price:size` joined with
//! colons, bids and asks interleaved — is OKX's alone, so a consumer reading
//! that field would have to branch on the venue to know what it meant. The
//! wire carries conclusions, not evidence (`documents/feed_handler.md` §8).
//!
//! Reversing this is cheap if the consumer turns out to want it: it fits the
//! [`SnapshotDeltaPayload`](jeed_wire::SnapshotDeltaPayload) padding and a
//! `delta_flags` bit, with no change to any other field.

pub mod book;
pub mod trade;

/// Bit positions in an OKX decoder's [`crate::mask`] word.
pub(crate) mod seen {
    /// `bids`.
    pub const BIDS: u8 = 1 << 0;
    /// `asks`.
    pub const ASKS: u8 = 1 << 1;
    /// `seqId`.
    pub const SEQUENCE: u8 = 1 << 2;
    /// `prevSeqId`.
    pub const PREV_SEQUENCE: u8 = 1 << 3;
    /// `px`.
    pub const PRICE: u8 = 1 << 4;
    /// `sz`.
    pub const QTY: u8 = 1 << 5;
}
