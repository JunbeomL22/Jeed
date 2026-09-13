//! `jeed_crypto::recv` — one test target whose module tree mirrors `src/recv`.
//!
//! The Binance frames are the decoder tests' own, reached by `#[path]` rather
//! than copied: a receive-loop test that spelled its own JSON would be testing
//! the spelling and not the loop (`CLAUDE.md`).

#[allow(dead_code)]
#[path = "../binance/common/mod.rs"]
mod frames;

mod endpoint;
mod live;
mod pipeline;
mod receiver;
mod router;
mod sink;
mod ws;
mod http;
mod route;
mod venue;
