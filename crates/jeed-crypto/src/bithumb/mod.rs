//! Bithumb market data (`wss://ws-api.bithumb.com/websocket/v1`).
//!
//! ```text
//! orderbook  → upbit::snapshot  WireKind::Quote        depth ≤ 10
//! trade      → upbit::trade     WireKind::Trade
//! ```
//!
//! ## The decoders are Upbit's, and that is not a shortcut
//!
//! Bithumb's public v2 WebSocket API is Upbit's, field for field: the same
//! channel names, the same `type`/`code`/`orderbook_units` shape, the same
//! paired bid/ask units, the same DEFAULT and SIMPLE key spellings, prices and
//! sizes as unquoted JSON numbers, and no diff stream. Writing a second copy
//! of [`upbit::snapshot`](crate::upbit::snapshot) here would produce two files
//! that have to be corrected together and will not be — the same reason
//! `jeed_krx::decode::derivative` keeps 주식파생 and 지수파생 in one module
//! (`CLAUDE.md`).
//!
//! ## What is actually different is the venue byte, and it matters
//!
//! `KRW-BTC` is a market on both exchanges and the two books have different
//! prices in them. Identity on the wire is `(venue, symbol)`, so the whole of
//! the difference between a Bithumb record and an Upbit one is
//! [`Venue::Bithumb`](jeed_wire::Venue::Bithumb) in the header — which the
//! decoder takes from the [`Instrument`](crate::Instrument) it is handed, not
//! from the module it lives in:
//!
//! ```
//! use jeed_crypto::{Instrument, bithumb};
//! use jeed_wire::{Venue, WireRecord};
//!
//! let inst = Instrument::new(Venue::Bithumb, b"KRW-BTC", 0, 8)?;
//! let mut rec = WireRecord::zeroed();
//! bithumb::trade::decode(&inst, br#"{"type":"trade","code":"KRW-BTC",
//!     "trade_price":152430000.0,"trade_volume":0.0084,"ask_bid":"BID"}"#,
//!     1_700_000_000_000_000_000, &mut rec)?;
//!
//! assert_eq!(rec.header.venue(), Ok(Venue::Bithumb));
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! Pass a [`Venue::Upbit`](jeed_wire::Venue::Upbit) instrument to
//! [`bithumb::trade`](trade) and you get an Upbit record — the module path is
//! documentation, and the instrument is the truth. That is the same property
//! every decoder in this crate has, and here it is the only thing standing
//! between the two exchanges, so it is worth saying out loud.
//!
//! ## If the two ever diverge
//!
//! Then Bithumb gets its own `snapshot.rs` and `trade.rs`, and this re-export
//! goes away. Until they do, one copy is what stays correct.

pub use crate::upbit::snapshot;
pub use crate::upbit::trade;
