//! Bitget V2 market data (`wss://ws.bitget.com/v2/ws/public`).
//!
//! ```text
//! books, books1, books5, books15  → book      Quote or SnapshotDelta
//! trade                           → trade     WireKind::Trade, batched
//! /api/v2/*/market/merge-depth    → snapshot  Quote (REST body)
//! ```
//!
//! ## OKX's message, with OKX's spelling changed
//!
//! Bitget's public feed is shaped exactly like OKX's — an `arg` envelope
//! naming the subscription, a `data` array holding one object, an `action`
//! that says whether the book is whole or a diff, and a sequence number that
//! names its predecessor. Only the words differ:
//!
//! | | OKX | Bitget |
//! |---|---|---|
//! | sequence | `seqId` / `prevSeqId` | `seq` / `pseq` |
//! | trade price · size | `px` · `sz` | `price` · `size` |
//! | level | `[px, sz, "0", orders]` | `[price, size]` |
//! | market | `instId` suffix (`-SWAP`) | `arg.instType` |
//!
//! The decoders are still separate files rather than one parameterised by a
//! key table: a table would make the two venues' *current* agreement a
//! structure, and the next protocol change to either would have to break it.
//!
//! ## Two venue bytes, one set of decoders
//!
//! [`BitgetSpot`](jeed_wire::Venue::BitgetSpot) and
//! [`BitgetLinear`](jeed_wire::Venue::BitgetLinear) carry the same messages —
//! `instType` is `SPOT` or `USDT-FUTURES` and nothing else in the frame
//! changes. They are two bytes because both markets spell the pair `BTCUSDT`,
//! so `(venue, symbol)` would name one instrument for two different books
//! (`CLAUDE.md`). Which byte a record gets is the [`Instrument`]'s to say, not
//! this module's — as with [`bithumb`](crate::bithumb).
//!
//! `instType` itself is therefore **not read**. It says which subscription
//! this is, and the caller already knew that when it built the instrument; a
//! decoder that checked it could only ever repeat the caller back to itself.
//!
//! ## `checksum` is not carried
//!
//! Every `books` message ships a CRC32 over the top twenty-five levels, and it
//! is dropped for the reasons [`okx`](crate::okx) gives — the handler holds no
//! book, so it cannot check it, and the rule for computing it is the venue's
//! own. Bitget makes the first half of that concrete: `fractal-engine`'s
//! validator has to *keep a book* to use the field at all, which is the one
//! thing a feed handler does not do.
//!
//! [`Instrument`]: crate::Instrument

pub mod book;
#[cfg(feature = "recv")]
pub mod router;
pub mod snapshot;
pub mod trade;

/// Bit positions in a Bitget decoder's [`crate::mask`] word.
pub(crate) mod seen {
    /// `bids`.
    pub const BIDS: u8 = 1 << 0;
    /// `asks`.
    pub const ASKS: u8 = 1 << 1;
    /// `seq`.
    pub const SEQUENCE: u8 = 1 << 2;
    /// `pseq`.
    pub const PREV_SEQUENCE: u8 = 1 << 3;
    /// `price`.
    pub const PRICE: u8 = 1 << 4;
    /// `size`.
    pub const QTY: u8 = 1 << 5;
}
