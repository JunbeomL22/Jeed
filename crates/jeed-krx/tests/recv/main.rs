//! `jeed_krx::recv` — one test target whose module tree mirrors `src/recv`.
//!
//! The message builders are the decoder tests' own, reached by path rather than
//! copied: a receive-loop test that spells its own 전문 would be testing the
//! spelling and not the loop (`CLAUDE.md`).

// Only the 파생 and 장운영 builders are used here; the module re-exports every
// market's.
#[allow(unused_imports)]
#[path = "../decode/common/mod.rs"]
mod builders;

mod endpoint;
mod filter;
mod pipeline;
mod sink;
mod stats;

mod receiver;
mod socket;
