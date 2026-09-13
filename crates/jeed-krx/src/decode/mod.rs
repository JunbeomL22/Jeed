//! Message decoders — raw datagram to [`jeed_wire::WireRecord`].
//!
//! A decoder never allocates, never keeps state between messages, and never
//! partially updates its output: on any error the caller's record is left as it
//! was, so a half-filled record cannot reach the ring (`CLAUDE.md`).
//!
//! ## Shape
//!
//! ```ignore
//! pub fn decode(payload: &[u8], recv_ns: UnixNano, out: &mut WireRecord)
//!     -> Result<(), KrxError>
//! ```
//!
//! The output buffer belongs to the caller — a ring slot in the binary, a
//! `Vec<WireRecord>` in a test. That is what keeps this crate free of
//! `jeed-shm`.

pub mod common;
pub mod derivative;
pub mod dispatch;
pub mod bond;
pub mod schedule;
pub mod securities;
pub mod etf;
pub mod stock;
