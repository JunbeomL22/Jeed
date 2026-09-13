//! What filling a book told us about it.
//!
//! The same two facts `jeed_krx::decode::common::BookShape` carries, worked
//! out the same way, because the wire header means the same thing whichever
//! feed filled it: `depth` is **levels actually carrying quantity**, not
//! levels the channel is capable of.
//!
//! Crypto reaches that rule from the opposite direction. KRX pads a five-deep
//! book out to five with zeros, so depth has to be counted downward from the
//! channel's capacity. A Binance snapshot simply stops, so depth is however
//! many arrived — until a partial-book stream on a thin instrument sends a
//! zero-quantity level, and then counting is the only thing that is right.

use jeed_wire::header_flags;

/// Depth and emptiness, ready for the record header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BookShape {
    /// Levels up to and including the deepest one carrying quantity, taken
    /// across both sides.
    pub depth: u8,

    /// [`header_flags::BID_EMPTY`] / [`header_flags::ASK_EMPTY`] as they apply.
    pub flags: u8,
}

impl BookShape {
    /// Shape of a book whose sides reached `bid_depth` and `ask_depth`.
    #[inline]
    pub const fn new(bid_depth: u8, ask_depth: u8) -> Self {
        let mut flags = 0u8;
        if bid_depth == 0 {
            flags |= header_flags::BID_EMPTY;
        }
        if ask_depth == 0 {
            flags |= header_flags::ASK_EMPTY;
        }
        let depth = if bid_depth > ask_depth { bid_depth } else { ask_depth };
        Self { depth, flags }
    }

    /// Shape of a top-of-book update, from the two quantities.
    #[inline]
    pub const fn top_of_book(bid_qty: u64, ask_qty: u64) -> Self {
        Self::new((bid_qty > 0) as u8, (ask_qty > 0) as u8)
    }
}
