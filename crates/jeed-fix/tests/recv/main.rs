//! `jeed_fix::recv` — one test target whose module tree mirrors `src/recv`.
//!
//! The message builders are the protocol tests' own, reached by `#[path]`
//! rather than copied: a receive-loop test that spelled its own FIX bytes would
//! be testing the spelling and not the loop (`CLAUDE.md`).

#[allow(unused_imports)]
#[path = "../fix/builders.rs"]
mod builders;

mod adapter;
mod emit;
mod endpoint;
mod filter;
mod pipeline;
mod receiver;
mod sink;
