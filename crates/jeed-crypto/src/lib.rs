//! Crypto exchange market data — JSON in, [`WireRecord`] out.
//!
//! ```text
//! [websocket frame]
//!      ↓ scan            json.rs        one pass, no allocation, no serde
//!      ↓ decode          binance/…      → &mut WireRecord
//! [wire record]
//! ```
//!
//! ## One crate, not one per exchange
//!
//! WebSocket + JSON is a protocol; Binance, Upbit, Bybit and the rest are
//! dialects of it. They differ in key names and in which fields a message
//! carries — not in how a message is scanned, how a decimal string becomes an
//! integer, or how a book fills a [`QuotePayload`]. Splitting the exchanges
//! into crates would put five copies of [`json`] on five compile units and let
//! them drift; the same reasoning that keeps 주식파생 and 지수파생 in one
//! `jeed_krx::decode::derivative` (`CLAUDE.md`).
//!
//! So the seam is [`Instrument`] plus a module per exchange, and the thing
//! that would justify a second crate — a transport, a session, a socket — is
//! not here at all.
//!
//! ## Where this crate stops
//!
//! At the record. There is no socket, no TLS, no reconnect and no thread:
//! `jeed-fix` stopped at [`MdMessage`]-equivalent for a while for the same
//! reason, and the receive loop is a separate piece of work
//! (`documents/todo.md`). Everything here runs against a byte slice, so the
//! tests are captured frames and nothing else.
//!
//! ## What crypto forces that KRX did not
//!
//! | | KRX | crypto |
//! |---|---|---|
//! | 프레이밍 | fixed length per trcode, `0xFF` terminator | JSON, variable |
//! | 종목 | ISIN in the message | the stream you subscribed to |
//! | 스케일 | product group + ISIN prefix decide it | per-symbol, from `exchangeInfo` |
//! | 북 | full snapshot every time | **incremental** — [`WireKind::SnapshotDelta`] |
//!
//! The last row is the one with teeth. Every KRX channel replaces the book it
//! describes, so a lost message costs an instant. A depth diff *modifies*, so
//! a lost one is wrong forever unless the consumer notices and resynchronises
//! — which is why the venue's update-id chain rides through this crate
//! untouched, and why a delta that will not fit the wire record is dropped
//! rather than truncated. See [`SnapshotDeltaPayload`].
//!
//! [`WireRecord`]: jeed_wire::WireRecord
//! [`QuotePayload`]: jeed_wire::QuotePayload
//! [`WireKind::SnapshotDelta`]: jeed_wire::WireKind::SnapshotDelta
//! [`SnapshotDeltaPayload`]: jeed_wire::SnapshotDeltaPayload
//! [`MdMessage`]: https://docs.rs/jeed-fix

#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod binance;
pub mod bithumb;
pub mod book;
pub mod bybit;
pub mod error;
pub mod instrument;
pub mod json;
pub mod okx;
pub mod upbit;

pub(crate) mod delta;
pub(crate) mod mask;

pub use book::BookShape;
pub use error::CryptoError;
pub use instrument::Instrument;

/// Milliseconds, as every crypto venue timestamps, in nanoseconds.
///
/// Saturating rather than wrapping: a garbage millisecond field must not come
/// out the far side as a plausible time. `u64::MAX` ns is year 2554, so this
/// only ever triggers on nonsense.
#[inline]
pub const fn millis_to_nanos(ms: u64) -> jeed_wire::UnixNano {
    ms.saturating_mul(1_000_000)
}
