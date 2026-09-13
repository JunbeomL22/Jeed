//! `jeed_fix` — one test target whose module tree mirrors `src/`.
//!
//! Cargo compiles every top-level file in `tests/` as its own crate, which is
//! why these live under one root (`CLAUDE.md`): the [`builders`] are shared
//! rather than copied per file, and `tests/recv` reaches them by `#[path]`.
//!
//! Those builders are a third, deliberately naive encoder. The library must
//! not be used to produce the bytes it is being tested on, and the capture the
//! `capture` module replays was written by a fourth one — `apps/smbs_to_fix`
//! in fractal-engine — so agreement there is evidence rather than a tautology
//! (`documents/feed_handler.md` §12).

mod builders;
mod capture;
mod error;
mod frame;
mod market_data;
mod session;
mod tagvalue;

pub use builders::*;
