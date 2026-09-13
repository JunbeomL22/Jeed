//! KuCoin spot Classic API (`wss://ws-api-spot.kucoin.com/`).
//!
//! ```text
//! /market/level2  → delta     WireKind::SnapshotDelta
//! /market/match   → trade     WireKind::Trade, one per frame
//! /api/v3/market/orderbook/level2  → snapshot  Quote (REST body)
//! ```
//!
//! ## The envelope
//!
//! ```json
//! {"type":"message","topic":"/market/level2:BTC-USDT","subject":"trade.l2update",
//!  "data":{…}}
//! ```
//!
//! `type` is `message` on data and `welcome` / `ack` / `pong` on everything
//! else the socket carries, and it is not read: a control frame has no `data`
//! worth decoding and says so by failing on the fields that matter, the way
//! [`gate`](crate::gate)'s `event` is left alone.
//!
//! ## The book channel is diffs only
//!
//! `/market/level2` never sends a whole book. KuCoin's own procedure is to
//! subscribe, buffer, fetch `/api/v3/market/orderbook/level2` and splice — so
//! [`delta`] produces [`WireKind::SnapshotDelta`](jeed_wire::WireKind::SnapshotDelta)
//! and the book it applies to comes from [`snapshot`].

pub mod delta;
pub mod snapshot;
pub mod trade;
