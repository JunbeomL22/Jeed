//! `jeed_krx::decode` — one test target whose module tree mirrors `src/decode`.
//!
//! Cargo compiles every top-level file in `tests/` as its own crate, which is
//! why the decoder tests live under one root here instead: the tree can then
//! follow `src/` exactly (`CLAUDE.md`), and the message builders are shared
//! rather than copied per file.

mod bond;
mod common;
mod derivative;
mod dispatch;
mod etf;
mod schedule;
mod securities;
mod stock;
