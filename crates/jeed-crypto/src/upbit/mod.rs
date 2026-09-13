//! Upbit market data (`wss://api.upbit.com/websocket/v1`).
//!
//! ```text
//! orderbook  → snapshot  WireKind::Quote        depth ≤ 10
//! trade      → trade     WireKind::Trade
//! ```
//!
//! Spot only, and not by choice of scope: a domestic Korean exchange may not
//! list crypto derivatives, so there is no futures market to add later and no
//! second [`Venue`](jeed_wire::Venue) byte waiting the way Binance and Bybit
//! have one.
//!
//! ## Four things that are not like Binance
//!
//! 1. **Numbers are numbers.** `"trade_price":52430000.0`, unquoted, where
//!    every other venue in this crate sends `"52430000.0"`. Handled by
//!    [`parse_scalar_bytes`](crate::json::parse_scalar_bytes) rather than by a
//!    separate set of decoders.
//! 2. **Two spellings of every key.** A subscription can ask for `isOnlySnapshot`
//!    style DEFAULT keys or the SIMPLE abbreviations (`ty`, `cd`, `tms`, `obu`,
//!    `ap`), and the venue then uses that spelling for the whole connection.
//!    Both are matched, so a decoder does not need to know which was asked
//!    for. That also rules out first-byte key dispatch: `ask_price` and
//!    `ask_size` share one, as do `ap` and `as`.
//! 3. **The book is snapshot-only.** There is no diff stream, so there is no
//!    [`SnapshotDelta`](jeed_wire::WireKind::SnapshotDelta) here and nothing
//!    for a consumer to resynchronise.
//! 4. **The two sides arrive paired.** One `orderbook_units` entry carries the
//!    bid *and* the ask at that rank, rather than a `bids` array and an `asks`
//!    array.
//!
//! ## There is no sequence number, and none is invented
//!
//! Upbit publishes no book sequence and no update id — `timestamp` is the only
//! monotonic-ish field on an orderbook message. `fractal-engine` echoed that
//! timestamp into the sequence slot; this does not, and leaves
//! [`quote_ext`](jeed_wire::quote_ext) at `NONE`. A millisecond clock in a
//! field labelled *sequence* is a consumer inviting itself to compare two
//! books that were published in the same millisecond and conclude they are the
//! same book. Absent is a fact; a stand-in is not (`documents/feed_handler.md`
//! §8).

pub mod snapshot;
pub mod trade;

/// Bit positions in an Upbit decoder's [`crate::mask`] word.
pub(crate) mod seen {
    /// `orderbook_units` / `obu`.
    pub const UNITS: u8 = 1 << 0;
    /// `trade_price` / `tp`.
    pub const PRICE: u8 = 1 << 1;
    /// `trade_volume` / `tv`.
    pub const QTY: u8 = 1 << 2;
    /// `ask_bid` / `ab`.
    pub const SIDE: u8 = 1 << 3;
}
