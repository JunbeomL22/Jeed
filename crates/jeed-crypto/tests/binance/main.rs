//! `jeed_crypto::binance` — the decoders, against the frames Binance sends.
//!
//! One test crate with a tree rather than a file per stream, so the frames and
//! the instruments live in one place (`CLAUDE.md`).

mod common;
mod futures;
mod spot;
