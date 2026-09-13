//! The feed-handler binaries' shared half.
//!
//! ```text
//! conf/*.toml ──→ toml ──→ conf ──→ validate
//!                                      │
//!            boot_id ─┐                ▼
//!    RingProducer ────┼──→ Receiver ──→ thread, pinned ──→ loop until signal
//!    sockets ─────────┘                     │
//!                                           └──→ report line, every N s
//! ```
//!
//! Each handler crate (`jeed-krx`, `jeed-fix`, `jeed-crypto`) owns its
//! receive loop and writes into a `RecordSink` it does not own. What is left
//! for a binary is the same for all three — read the conf, create the
//! segments, pin the threads, wire the sink to a ring, run until told to stop
//! (`documents/todo.md` §2) — and that is what lives here. `src/bin/krx.rs`
//! and `src/bin/crypto.rs` are the few dozen lines that are each handler's
//! alone; [`krx`] and [`crypto`] are their wiring, [`feed`] what the two
//! wirings share.
//!
//! ## What is deliberately not here
//!
//! - A TOML crate, a logging crate, a signal crate, an affinity crate. Each is
//!   a few hundred lines of cold path at most, and the workspace has one
//!   external dependency (`rustls`) for a reason it was worth arguing about.
//! - A watchdog thread. The report line, like the heartbeat, is written by the
//!   receive thread itself — a separate thread would keep describing a loop
//!   that had stopped (`documents/feed_handler.md` §9).

#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod boot;
pub mod conf;
pub mod cpu;
pub mod crypto;
pub mod feed;
pub mod krx;
pub mod log;
pub mod signal;
pub mod toml;

pub use boot::boot_id;
pub use conf::{ConfError, CryptoConf, KrxConf, TrCodeTable};
pub use cpu::{CpuError, Topology};
pub use feed::Options;
