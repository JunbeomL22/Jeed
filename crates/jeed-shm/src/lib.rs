//! Shared-memory transport for [`jeed_wire`] records.
//!
//! One segment is one SPSC channel: an anonymously backed named shared-memory
//! object — the pagefile on Windows, `/dev/shm` on Linux —
//! holding a [`SegmentHeader`](jeed_wire::SegmentHeader) followed by a power-of-two
//! array of fixed-size record slots. The feed handler writes
//! ([`RingProducer`]); the consumer process reads
//! ([`RingConsumer`]), read-only and with its own cursor.
//!
//! ```text
//! offset 0      SegmentHeader   128 B   magic · version · capacity · boot_id
//!                                       | write_cursor · drop_counter
//! offset 128    slot 0          640 B
//!               slot 1          640 B
//!               …               capacity slots, each 10 cache lines
//! ```
//!
//! The ring **overwrites** rather than back-pressures, and the record's own
//! `producer_seq` doubles as a sequence lock. Both choices are argued in
//! [`ring`]; they are the two things to understand before changing anything
//! here.
//!
//! ## Shape of a session
//!
//! ```no_run
//! use jeed_shm::{Recv, RingConsumer, RingProducer, SegmentName};
//! use jeed_wire::WireRecord;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Feed handler, on its pinned core.
//! let name = SegmentName::local("jeed.krx.hot")?;
//! let mut tx = RingProducer::create(&name, 1 << 16, 0xfeed_0001)?;
//! # let decoded_ok = |_: &mut WireRecord| true;
//! let mut slot = tx.slot();
//! if decoded_ok(&mut slot) {
//!     slot.commit();
//! } // else: dropped, and nothing is published
//!
//! // Consumer process.
//! let mut rx = RingConsumer::attach(&name)?;
//! let mut rec = WireRecord::zeroed();
//! match rx.try_recv(&mut rec) {
//!     Recv::Record => { /* rec.header.producer_seq is the ordering authority */ }
//!     Recv::Empty => { /* quiet, or dead — only heartbeats tell them apart */ }
//!     Recv::Lagged(n) => { /* n records lost; book is a snapshot behind */ }
//!     Recv::Restarted { boot_id: _ } => { /* producer rebooted; book is stale */ }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Platform
//!
//! Windows and Linux. `CreateFileMappingW` + `MapViewOfFile` against the
//! pagefile on one, `shm_open` + `mmap` on the other; [`ring`], [`producer`]
//! and [`consumer`] sit above the difference and contain no platform code.
//!
//! **One rule does not carry over**: a Windows section dies with its last
//! handle, a POSIX object is a name in `/dev/shm` that outlives every process
//! that touched it. What that changes, and what it does not, is in [`mapping`].

#![cfg(any(windows, target_os = "linux"))]
#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod consumer;
pub mod error;
pub mod mapping;
pub mod name;
pub mod producer;
pub mod ring;

pub use consumer::{Recv, RingConsumer};
pub use error::ShmError;
pub use mapping::{Access, SharedMapping};
pub use name::{MAX_NAME_LEN, SegmentName};
pub use producer::{RingProducer, Slot};
pub use ring::SEQ_IN_PROGRESS;

use jeed_wire::{SEGMENT_HEADER_LEN, WIRE_RECORD_LEN};

/// Bytes a segment of `capacity` slots occupies.
///
/// `capacity` should be a power of two; [`RingProducer::create`] rejects
/// anything else.
#[inline]
pub const fn segment_len(capacity: u64) -> usize {
    SEGMENT_HEADER_LEN + capacity as usize * WIRE_RECORD_LEN
}
