//! The one path a finished record takes out of a handler.
//!
//! ```text
//! [datagram] ─decode→ &mut WireRecord ─commit→ [ring slot]
//!                     └──────────── RecordSink ─────────┘
//! ```
//!
//! ## Why the trait lives in the ABI crate
//!
//! `jeed-krx` must not depend on `jeed-shm` — a decoder bug must not be able to
//! reach the segment, which is why decoders write into a `&mut WireRecord` the
//! caller owns. But the receive loop *does* publish, and `jeed-fix` will
//! publish the same way; if each handler declared its own sink trait, the
//! orphan rule would force each to carry its own adapter and "one path to the
//! ring" would quietly become two.
//!
//! Declaring it here costs nothing: this crate has no dependencies, both
//! handlers already link it, and `jeed-shm` implements it for `RingProducer`.
//! The handlers stay generic over `S: RecordSink` and never name the transport.
//!
//! ## Why `fill`, and not `publish(&WireRecord)`
//!
//! The record is built **in the slot**. Handing the sink a finished record
//! would mean staging it somewhere first and copying 640 bytes per message for
//! nothing.
//!
//! The signature also makes the rule from `CLAUDE.md` — *never publish a
//! half-filled record* — a property of the type rather than of the caller's
//! discipline. A `fill` that returns `Err` publishes nothing; there is no way
//! to reach a slot except through a closure whose result decides its fate.

use crate::WireRecord;

/// Somewhere a completed [`WireRecord`] can be published.
///
/// Implemented by `jeed_shm::RingProducer` for the live path, and by whatever a
/// test or a replay wants to collect into.
///
/// Not object-safe, deliberately: [`publish`](Self::publish) is generic over
/// the filler's error so a handler can hand its own error type straight
/// through. Nothing needs `dyn` — the receive loop is generic over one sink for
/// its whole life.
pub trait RecordSink {
    /// Claims a record buffer, runs `fill` on it, and publishes it **only** if
    /// `fill` returns `Ok`.
    ///
    /// On `Err` the buffer is abandoned with whatever `fill` managed to write
    /// still in it, and nothing observes it.
    fn publish<E>(&mut self, fill: impl FnOnce(&mut WireRecord) -> Result<(), E>)
    -> Result<(), E>;

    /// Notes `n` records dropped *before* a buffer was claimed — a datagram
    /// that failed its frame check, or one a filter rejected.
    ///
    /// Nothing inside the ring drops: it overwrites. This counter is for the
    /// records that never got that far.
    fn note_drops(&mut self, n: u64);
}
