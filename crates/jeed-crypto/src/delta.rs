//! Filling a [`SnapshotDeltaPayload`]'s shared level array from two sides
//! that arrive in whatever order the venue felt like.
//!
//! The wire lays bids out first, then asks, in one array
//! ([`SnapshotDeltaPayload::levels`]). Every venue that sends a diff sends the
//! two sides as separate keys, and every one of them happens to send bids
//! first — except OKX, which sends `asks` first. Assuming a key order is not
//! something a decoder gets to do anyway, and the cost of not assuming it is
//! one [`rotate_left`] on a slice that is already sitting in the record.
//!
//! [`rotate_left`]: slice::rotate_left

use crate::error::CryptoError;
use crate::instrument::Instrument;
use crate::json::delta_levels;
use jeed_wire::{SnapshotDeltaPayload, WireDeltaLevel};

/// Accumulates the two sides into one shared array in whichever order they
/// arrive.
pub(crate) struct Sides {
    /// Levels written so far, from index 0.
    pub written: usize,

    /// How many of them are bids.
    pub bid_count: usize,

    /// How many of them are asks.
    pub ask_count: usize,

    /// Whether the asks landed first and so need rotating out of the way.
    ask_first: bool,
}

impl Sides {
    /// Nothing written yet.
    pub(crate) const fn new() -> Self {
        Self { written: 0, bid_count: 0, ask_count: 0, ask_first: false }
    }

    /// Reads one side's array into the free tail of `levels`.
    ///
    /// Returns the position past the array. Overflow is
    /// [`CryptoError::DeltaOverflow`] and the frame is lost, which is the
    /// point — see [`SnapshotDeltaPayload`].
    pub(crate) fn read(
        &mut self,
        inst: &Instrument,
        payload: &[u8],
        pos: usize,
        levels: &mut [WireDeltaLevel],
        is_bid: bool,
    ) -> Result<usize, CryptoError> {
        if !is_bid && self.written == 0 && self.bid_count == 0 {
            self.ask_first = true;
        }
        let (n, next) = delta_levels(inst, payload, pos, &mut levels[self.written..])?;
        self.written += n;
        if is_bid {
            self.bid_count = n;
        } else {
            self.ask_count = n;
        }
        Ok(next)
    }

    /// Puts the bids in front, if they were not already, and writes the counts.
    pub(crate) fn finish(&self, d: &mut SnapshotDeltaPayload) {
        if self.ask_first {
            d.levels[..self.written].rotate_left(self.ask_count);
        }
        d.bid_count = self.bid_count as u8;
        d.ask_count = self.ask_count as u8;
    }
}
