//! Wire records — the fixed-layout ABI between the feed handler process and
//! its consumer (`documents/feed_handler.md` §4–§8).
//!
//! Three layers are kept apart on purpose:
//!
//! ```text
//! [raw bytes]      KRX UDP / FIX tag-value / parquet row
//!       ↓ decode                       (feed handler, backtest reader)
//! [wire record]    #[repr(C)] · fixed size · versioned · shm ABI   ← this crate
//!       ↓ adapt                        (consumer entry point, one implementation)
//! [normalized]     the consumer's own in-process Rust types
//! ```
//!
//! The wire record is the *only* thing two independently deployed binaries
//! agree on. It therefore carries no interned identifiers (those are
//! process-local), no Rust enums with implicit layout, and no `Option`; every
//! field is a plain integer or byte array with an explicit width, and every
//! struct has explicit padding so there are no uninitialised bytes. Layout
//! invariants are enforced with compile-time assertions in [`record`] and
//! [`segment`]; a mismatch fails the build, not the market.
//!
//! **This crate has no dependencies, deliberately.** It is what the consumer
//! links against, so anything it pulls in, the consumer pulls in too — and a
//! decoder bug must never force the consumer to redeploy.

#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]

/// Wire error type (no heap-allocating fields).
pub mod error;
/// Fixed 48-byte record header shared by every record kind.
pub mod header;
/// Record kinds and flag bit definitions.
pub mod kind;
/// Per-kind payload structs and the payload union.
pub mod payload;
/// The fixed-size record and its byte views.
pub mod record;
/// Shared-memory segment header (written once by the producer at boot).
pub mod segment;
/// Value types carried on the wire.
pub mod types;

pub use error::WireError;
pub use header::RecordHeader;
pub use kind::{
    WireKind, expansion_direction, header_flags, level_ext, level_flags, quote_ext, schedule_flags,
    trade_flags, trade_kind,
};
pub use payload::{
    HeartbeatPayload, InvestorStatsPayload, MarketSchedulePayload, OpenInterestPayload,
    PriceLimitPayload, QuotePayload, TradePayload, TradeQuotePayload, WireLevel, WirePayload,
};
pub use record::WireRecord;
pub use segment::{SEGMENT_HEADER_LEN, SEGMENT_MAGIC, SegmentHeader};
pub use types::{BookPrice, BookQuantity, BookYield, ISIN_LEN, Isin, OrderCount, Scale, UnixNano, Venue};

/// Wire format version. Bump on **any** layout or semantic change; the reader
/// rejects a segment whose version differs ([`SegmentHeader::check`]).
///
/// Bumping this is a **both-binaries-at-once deployment event**, not a
/// refactor. Until a consumer is attached the layout is still free to move.
pub const WIRE_FORMAT_VERSION: u32 = 1;

/// Maximum book depth carried per side.
///
/// KRX stock / ETF / stock-derivative channels are 10-deep; index-derivative
/// and bond channels use 5 of the 10.
pub const WIRE_MAX_DEPTH: usize = 10;

/// Size of the fixed record header in bytes.
pub const WIRE_HEADER_LEN: usize = 48;

/// Size of the payload area in bytes — sized by the largest payload
/// ([`TradeQuotePayload`], 32 + 496).
pub const WIRE_PAYLOAD_LEN: usize = 528;

/// Size of one record in bytes: header + payload + explicit tail padding,
/// rounded up to a whole number of 64-byte cache lines (9 lines; 48 + 528
/// needs no tail).
pub const WIRE_RECORD_LEN: usize = 576;

/// Alignment of a record and of the segment header (one cache line).
pub const WIRE_ALIGN: usize = 64;
