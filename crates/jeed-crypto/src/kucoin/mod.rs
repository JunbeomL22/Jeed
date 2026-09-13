//! KuCoin Classic API market data.
//!
//! ```text
//! spot     wss://ws-api-spot.kucoin.com   /market/level2 · /market/match
//! futures  wss://ws-api-futures.kucoin.com  /contractMarket/level2 · /contractMarket/execution
//! ```
//!
//! ## Two markets, and almost nothing shared between them
//!
//! [`spot`] and [`futures`] are separate modules because KuCoin's two APIs
//! agree on the envelope and on nothing inside it:
//!
//! | | spot | futures |
//! |---|---|---|
//! | book diff | `changes.{bids,asks}`, many levels | `change`, **one** level in a string |
//! | sequence | `sequenceStart` … `sequenceEnd` | `sequence`, one per message |
//! | trade size | quoted decimal | unquoted integer (contracts) |
//! | trade clock | `time`, nanoseconds | `ts`, nanoseconds |
//! | symbol | `BTC-USDT` | `XBTUSDTM` |
//!
//! That is the same rule `binance::{spot, futures}` follows: modules split
//! where the *message* splits.
//!
//! ## Two venue bytes, for a reason that is not a collision
//!
//! [`KucoinSpot`](jeed_wire::Venue::KucoinSpot) and
//! [`KucoinFutures`](jeed_wire::Venue::KucoinFutures) could have been one
//! byte on today's symbols — futures renames the asset and suffixes the
//! contract, so `XBTUSDTM` cannot collide with `BTC-USDT` the way Bybit's
//! `BTCUSDT` collides with itself. They are two anyway, because that
//! separation is a naming habit rather than a rule KuCoin documents, and the
//! cost of it turning out to be wrong is two order books merged into one
//! (`CLAUDE.md`).
//!
//! ## The topic carries the symbol
//!
//! Every frame names its subscription in `topic`, with the symbol after a
//! colon:
//!
//! ```text
//! /market/level2:BTC-USDT              →  BTC-USDT
//! /contractMarket/execution:XBTUSDTM   →  XBTUSDTM
//! ```
//!
//! Spot also repeats the symbol inside `data`, and futures book messages do
//! not — so [`topic_symbol`] is the only identity a futures diff has, and the
//! decoders check whichever the message carries.
//!
//! ## Timestamps are nanoseconds here, except when they are milliseconds
//!
//! KuCoin is the only venue in this crate that timestamps in nanoseconds, and
//! it does not do it everywhere: both trade channels send nanoseconds, the
//! futures book diff sends milliseconds, and the spot REST book sends
//! milliseconds. Each decoder says which it is reading, because the fields are
//! six orders of magnitude apart and a wrong guess dates a record to 1970 or
//! to the year 56000.

pub mod futures;
pub mod spot;

/// The symbol in a `/channel:SYMBOL` topic.
///
/// `None` when there is no colon or nothing after it — an unreadable identity
/// is the same situation as a wrong one, and the decoders treat it that way.
///
/// ```
/// use jeed_crypto::kucoin::topic_symbol;
///
/// assert_eq!(topic_symbol(b"/market/level2:BTC-USDT"), Some(&b"BTC-USDT"[..]));
/// assert_eq!(topic_symbol(b"/contractMarket/level2:XBTUSDTM"), Some(&b"XBTUSDTM"[..]));
/// assert_eq!(topic_symbol(b"/market/level2"), None);
/// ```
#[inline]
pub fn topic_symbol(topic: &[u8]) -> Option<&[u8]> {
    let colon = topic.iter().position(|&b| b == b':')?;
    let symbol = &topic[colon + 1..];
    if symbol.is_empty() { None } else { Some(symbol) }
}

/// Checks the symbol inside a `topic` value.
///
/// A topic that names no symbol is [`CryptoError::SymbolMismatch`] rather than
/// an ignored field: on the futures book channel it is the only identity the
/// frame has, so being unable to read it is the same situation as reading the
/// wrong one.
///
/// [`CryptoError::SymbolMismatch`]: crate::CryptoError::SymbolMismatch
pub(crate) fn check_topic(
    inst: &crate::Instrument,
    topic: &[u8],
) -> Result<(), crate::CryptoError> {
    match topic_symbol(topic) {
        Some(symbol) => inst.check_symbol(symbol),
        None => Err(crate::CryptoError::SymbolMismatch),
    }
}

/// Bit positions in a KuCoin decoder's [`crate::mask`] word.
pub(crate) mod seen {
    /// `bids`.
    pub const BIDS: u8 = 1 << 0;
    /// `asks`.
    pub const ASKS: u8 = 1 << 1;
    /// `sequence`, or spot's `sequenceStart`.
    pub const SEQUENCE: u8 = 1 << 2;
    /// Spot's `sequenceEnd`.
    pub const SEQUENCE_END: u8 = 1 << 3;
    /// `price`.
    pub const PRICE: u8 = 1 << 4;
    /// `size`.
    pub const QTY: u8 = 1 << 5;
    /// The futures diff's `change` string.
    pub const CHANGE: u8 = 1 << 6;
}
