//! KRX UDP multicast reception and message decoding into [`jeed_wire`] records.
//!
//! ```text
//! [UDP datagram]   fixed-width ASCII, 0xFF-terminated
//!       ↓ trcode dispatch          five bytes decide the layout
//!       ↓ frame check              length + end keyword, before any field
//!       ↓ decode                   this crate
//! [WireRecord]     handed to the caller's buffer — a ring slot, or a Vec in tests
//! ```
//!
//! ## What this crate does not do
//!
//! It does not own the segment. A decoder writes into a `&mut WireRecord` the
//! caller supplies, so it runs against a `Vec<WireRecord>` in a test and a ring
//! slot in the binary, and a decoder bug cannot reach shared memory.
//!
//! It does not normalize. The output is the wire record; turning that into the
//! consumer's own types happens in the consumer process
//! (`documents/feed_handler.md` §4).
//!
//! It keeps no book. The handler is stateless — decode, filter, publish.
//!
//! ## Two facts about KRX messages that shape everything here
//!
//! **The dispatch key is all five trcode bytes.** `B6` alone spans eight
//! interfaces from 324 to 1387 bytes; only the product group settles which.
//! See [`trcode`].
//!
//! **Blank is not zero.** 정보분배일련번호 arrives blank on entire channels, and
//! reading it as zero makes every message look like a sequence gap. See
//! [`field`].

#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]

/// Turning `HHMMSSuuuuuu` into an absolute timestamp.
pub mod clock;
/// Message decoders — raw datagram to [`jeed_wire::WireRecord`].
pub mod decode;
/// Decode errors.
pub mod error;
pub mod extract;
/// Fixed-width ASCII field readers.
pub mod field;
/// The frame check that runs before any field is read.
pub mod message;
/// 수신부 — UDP multicast in, one ring out.
pub mod recv;
/// The five-byte message type code.
pub mod trcode;

pub use error::KrxError;
pub use message::{END_KEYWORD, validate, validate_as};
pub use trcode::{TRCODE_LEN, TrCode};

use jeed_wire::Venue;

/// The venue every record from this crate carries.
pub const VENUE: Venue = Venue::Krx;
