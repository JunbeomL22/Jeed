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
//! ## Where the decoders stop, and where the loop begins
//!
//! The decoders stop at the record: every one of them runs against a byte
//! slice, so their tests are captured frames and nothing else. The socket,
//! TLS, reconnect and thread are [`recv`], behind the `recv` feature (on by
//! default) — a consumer that only wants the decoders turns it off and links
//! no TLS. That split is the reason this crate can have a transport at all
//! without contradicting the paragraph above: the transport is a module the
//! decoders do not see.
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
pub mod bitget;
pub mod bithumb;
pub mod book;
pub mod bybit;
pub mod clock;
pub mod error;
pub mod gate;
pub mod instrument;
pub mod json;
pub mod kucoin;
pub mod okx;
#[cfg(feature = "recv")]
pub mod recv;
pub mod time;
pub mod upbit;

pub(crate) mod delta;
pub(crate) mod mask;

pub use book::BookShape;
pub use error::CryptoError;
pub use instrument::Instrument;
pub use time::millis_to_nanos;
