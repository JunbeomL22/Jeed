//! Binance USD-M futures streams (`wss://fstream.binance.com`).
//!
//! ```text
//! @bookTicker      → bbo       WireKind::Quote        depth 1, with T
//! @aggTrade        → trade     WireKind::Trade
//! @depth10@100ms   → snapshot  WireKind::Quote        depth ≤ 10
//! /fapi/v1/depth   → snapshot  (a different envelope, same decoder)
//! @depth@100ms     → delta     WireKind::SnapshotDelta, with pu
//! ```
//!
//! ## Three differences from spot, all of them load-bearing
//!
//! 1. **`T`.** Every market-data message carries a transaction time as well
//!    as an event time, and `T` is the one that says when the exchange did
//!    the thing. Spot only has `E` on some streams and nothing on others, so
//!    a spot `@bookTicker` reaches the consumer with no venue time at all
//!    while this one does not.
//! 2. **`@aggTrade`, not `@trade`.** USD-M has no per-print stream; an
//!    aggregate trade is every fill at one price from one taker order, so
//!    `q` is their total and `f`/`l` bound the trade ids it replaces.
//! 3. **`pu`.** The depth diff names the message it must follow, so the
//!    consumer can verify the chain without assuming `U == last u + 1`. It
//!    rides through as
//!    [`prev_final_update_id`](jeed_wire::SnapshotDeltaPayload::prev_final_update_id).

pub mod bbo;
pub mod delta;
pub mod snapshot;
pub mod trade;
