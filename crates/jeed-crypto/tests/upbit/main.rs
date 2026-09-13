//! `jeed_crypto::upbit` — the decoders, against the frames Upbit sends.
//!
//! One test crate with a tree rather than a file per channel, so the frames
//! and the instruments live in one place (`CLAUDE.md`).

mod common;
mod snapshot;
mod trade;
