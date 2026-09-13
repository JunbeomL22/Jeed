//! FIX 4.4 market-data decoding — the protocol layer, and nothing about any
//! venue.
//!
//! ```text
//! [socket bytes]
//!      ↓ frame          frame.rs        8=…<SOH>9=len<SOH> … 10=ddd<SOH>
//! [Frame]                              BodyLength + CheckSum verified
//!      ↓ scan           tagvalue.rs     tag=value<SOH>, integers, UTC times
//!      ↓ decode         market_data.rs  35=W / 35=X → MdMessage (stack)
//!      ↓                session.rs      34 gaps, 35=0/1/2/3/4/5/A, liveness
//! [MdMessage]
//!      ↓ adapt          the venue adapter → WireRecord
//! ```
//!
//! **FIX is a protocol, not a place.** SMBS happens to be the first venue we
//! reach over it, so nothing here mentions SMBS — and unlike
//! `jeed_krx::VENUE`, this crate declares no venue at all. A second FIX
//! venue adds one adapter and changes nothing in here.
//!
//! That is also where this crate currently stops: the output is
//! [`MdMessage`], not a [`WireRecord`](jeed_wire::WireRecord). Turning a
//! decoded message into wire records takes venue knowledge — which symbol is
//! which instrument, what an absent size means, whether an incremental may
//! touch the book — and none of that is protocol
//! (`documents/feed_handler.md` §13, `documents/todo.md` §6).
//!
//! ## What the rest of Jeed relies on
//!
//! - **No allocation and no copying of message bodies.** [`Frame`] and
//!   [`Field`] borrow the receive buffer; [`MdMessage`] is a fixed-size value
//!   built on the stack.
//! - **Framing is validated before any field is read**, the same order the
//!   KRX decoders check length and end keyword (`CLAUDE.md`): `BodyLength` +
//!   `CheckSum` here is `KrxDataParse::validate` there.
//! - **Prices and sizes become integers at the parse site** on an explicit
//!   [`FixScales`], so the rounding point cannot drift between live and
//!   replay. A value with more fractional digits than the scale keeps is an
//!   error ([`FixError::Precision`]), never a silent round.
//! - **Venue time comes from `MDEntryDate` + `MDEntryTime` (272 / 273)**,
//!   never from `SendingTime` (52), which is the counterparty's clock.
//!
//! ## KRX and FIX are opposites — deliberately not shared
//!
//! `documents/feed_handler.md` §10: UDP multicast loses datagrams as a matter
//! of course and heals on the next snapshot, so KRX does not carry a sequence
//! check into the wire record. TCP does not lose messages, so a FIX gap is a
//! session incident that owes the counterparty a `ResendRequest`. Dropping
//! the sequence check on one side must not drop it on the other, which is why
//! [`session`] lives here and has no counterpart in `jeed-krx`.
//!
//! This crate owns no socket, spawns no thread, and never publishes: it
//! decodes bytes the caller hands it and answers what happened to the
//! session. The binary owns the connection and the ring.

#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]

/// The one place this crate reads a clock.
pub mod clock;
/// Errors raised while framing or decoding.
pub mod error;
/// Message framing and the streaming receive buffer.
pub mod frame;
/// `35=W` / `35=X` decoding into a fixed-capacity message.
pub mod market_data;
/// 수신부 — the TCP session, the loop, and the venue seam.
pub mod recv;
/// Sequence, administration, and liveness bookkeeping.
pub mod session;
/// Tag-value scanning and value parsers.
pub mod tagvalue;

pub use error::FixError;
pub use frame::{FIX_MAX_BODY_LENGTH, FIX_TRAILER_LEN, Frame, FrameBuffer, checksum, frame};
pub use market_data::{
    FIX_TEXT_LEN, FixScales, FixText, MD_MAX_ENTRIES, MdEntry, MdEntryType, MdMessage,
    MdUpdateAction, parse_md_message,
};
pub use recv::{
    Config, Emitter, Endpoint, Ingested, Link, LinkState, MdAdapter, Mode, Outcome, Pipeline,
    Receiver, Reply, SymbolFilter,
};
pub use session::{AdminMessage, SeqVerdict, SessionState, msg_type, parse_admin_message};
pub use tagvalue::{
    DecimalError, Field, Fields, MsgType, parse_scaled_decimal, parse_u64, parse_utc_date,
    parse_utc_time, parse_utc_timestamp, split_field,
};

/// Field separator (`SOH`, ASCII 1).
pub const SOH: u8 = 0x01;

/// `BeginString` of the only FIX version we speak.
///
/// [`frame()`] does not enforce it — framing must succeed before a version
/// policy can be applied to what it framed.
pub const FIX_BEGIN_STRING: &[u8] = b"FIX.4.4";
