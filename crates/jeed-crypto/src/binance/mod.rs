//! Binance market data.
//!
//! Two markets, two [`Venue`] bytes, one module each:
//!
//! | | [`spot`] | [`futures`] |
//! |---|---|---|
//! | venue | [`Venue::BinanceSpot`](jeed_wire::Venue::BinanceSpot) | [`Venue::BinanceFutures`](jeed_wire::Venue::BinanceFutures) |
//! | 호스트 | `wss://stream.binance.com:9443` | `wss://fstream.binance.com` |
//! | 체결 | `@trade` | `@aggTrade` — there is no `@trade` |
//! | 최우선호가 | `@bookTicker` | `@bookTicker`, plus `T` |
//! | 부분 북 | `@depth10@100ms` (`lastUpdateId` 봉투) | `@depth10@100ms` (`depthUpdate` 봉투) |
//! | 증분 | `@depth@100ms` | `@depth@100ms`, plus `pu` |
//!
//! ## They are two venues, not one venue with a flag
//!
//! `BTCUSDT` is a spot pair on one and a perpetual on the other. Identity on
//! the wire is `(venue, symbol)` (`documents/feed_handler.md` §6), so if the
//! two shared a venue byte they would share an identity, and a consumer would
//! merge two different instruments' books. Hence
//! [`Venue::BinanceFutures`](jeed_wire::Venue::BinanceFutures) as its own
//! byte — and COIN-M, when it arrives, as a third.
//!
//! ## The two markets are not merged the way the KRX derivatives were
//!
//! `CLAUDE.md` says to split a market module only where the message splits,
//! and `jeed_krx::decode::derivative` keeps 주식파생 and 지수파생 together
//! because the layouts are identical. Binance spot and USD-M are the other
//! case: the trade streams are different streams with different names and
//! different fields, the book envelopes differ, and the two markets version
//! independently. What they genuinely share — the scanner, the level arrays,
//! the instrument — is shared, and lives one level up.
//!
//! [`Venue`]: jeed_wire::Venue

pub mod futures;
#[cfg(feature = "recv")]
pub mod router;
pub mod spot;

/// `true` when the aggressor was the seller.
///
/// Binance reports `m` — *was the buyer the maker* — and the aggressor is
/// therefore the other party. Both markets spell it the same way.
#[inline]
pub(crate) const fn seller_aggressed(is_buyer_maker: bool) -> u8 {
    if is_buyer_maker { jeed_wire::trade_kind::SELL } else { jeed_wire::trade_kind::BUY }
}

/// Bit positions in a Binance decoder's [`crate::mask`] word.
pub(crate) mod seen {
    /// Bid price.
    pub const BID_PRICE: u8 = 1 << 0;
    /// Bid quantity.
    pub const BID_QTY: u8 = 1 << 1;
    /// Ask price.
    pub const ASK_PRICE: u8 = 1 << 2;
    /// Ask quantity.
    pub const ASK_QTY: u8 = 1 << 3;
    /// Trade price.
    pub const PRICE: u8 = 1 << 0;
    /// Trade quantity.
    pub const QTY: u8 = 1 << 1;
    /// Aggressor flag.
    pub const MAKER: u8 = 1 << 2;
    /// Bid side of a book message.
    pub const BIDS: u8 = 1 << 0;
    /// Ask side of a book message.
    pub const ASKS: u8 = 1 << 1;
    /// An update-id or sequence field.
    pub const SEQUENCE: u8 = 1 << 2;
}
