//! KuCoin futures Classic API (`wss://ws-api-futures.kucoin.com/`).
//!
//! ```text
//! /contractMarket/level2     → delta     WireKind::SnapshotDelta, one level
//! /contractMarket/execution  → trade     WireKind::Trade, one per frame
//! /api/v1/level2/snapshot    → snapshot  Quote (REST body)
//! ```
//!
//! ## Sizes are contracts, not coins
//!
//! A futures size is a whole number of contracts — `5`, unquoted — where spot
//! sends `"0.01022222"`. So an [`Instrument`](crate::Instrument) for this
//! venue takes **zero** quantity decimals, and the scale on the record says
//! so; a consumer that wants coins multiplies by the contract's multiplier,
//! which is instrument reference data and not something a market-data message
//! carries.
//!
//! ## The book channel is diffs only, one level at a time
//!
//! Where spot batches several changes into `changes.{bids,asks}`, futures
//! sends exactly one level per message, in a comma-separated string:
//!
//! ```text
//! "change":"90631.2,sell,2"
//! ```
//!
//! [`delta`] takes that apart; see its docs for why a string is not a problem
//! and an unreadable one is.

pub mod delta;
pub mod snapshot;
pub mod trade;
