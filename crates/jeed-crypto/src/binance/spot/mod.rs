//! Binance spot streams (`wss://stream.binance.com:9443`).
//!
//! ```text
//! @bookTicker      → bbo       WireKind::Quote        depth 1
//! @trade           → trade     WireKind::Trade
//! @depth10@100ms   → snapshot  WireKind::Quote        depth ≤ 10
//! /api/v3/depth    → snapshot  (the same envelope)
//! @depth@100ms     → delta     WireKind::SnapshotDelta
//! ```
//!
//! [`snapshot`] serves both the partial-book stream and the REST endpoint
//! because spot spells them identically — `lastUpdateId` and two arrays. That
//! is not true on USD-M, where the stream wears the `depthUpdate` envelope;
//! see [`futures::snapshot`](crate::binance::futures::snapshot).

pub mod bbo;
pub mod delta;
pub mod snapshot;
pub mod trade;
