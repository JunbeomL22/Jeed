//! `A3` — 파생 체결. `IFMSRPD0036`, 173 B.
//!
//! ```text
//! [0:47]     header
//! [47:172]   trade block — byte for byte the same block G7 carries
//! [172:173]  0xFF
//! ```
//!
//! `A3` is `G7` with the book cut off, so both read the block through
//! [`fill_trade`] rather than each
//! spelling the offsets out. That matters because the block ends with the
//! dynamic price limits, whose offsets are the ones the spec's end-offset
//! column gets wrong in a way that still parses.
//!
//! ## Prefer `G7` where both arrive
//!
//! Where a channel carries both, the `G7` form is the one to consume: the print
//! and the book it left behind cannot be separated by loss or reordering. `A3`
//! is for the channels that send nothing else, and for the moment before a
//! consumer has a book at all.

use crate::decode::derivative::{TRADE_BLOCK_END, fill_record_header, fill_trade, header};
use crate::error::KrxError;
use crate::extract;
use crate::trcode::TrCode;
use jeed_wire::{RecordHeader, UnixNano, Venue, WireKind, WireRecord};

/// `IFMSRPD0036` — the trade block plus the end keyword.
pub const MESSAGE_LEN: usize = TRADE_BLOCK_END + 1;

const _: () = assert!(MESSAGE_LEN == 173);

/// The `A3` 파생 체결 decoder. Stateless; one shape for every product group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DerivativeTrade;

/// `IFMSRPD0036`.
pub const DECODER: DerivativeTrade = DerivativeTrade;

impl DerivativeTrade {
    /// Fixed message length this interface defines.
    #[inline]
    pub const fn message_len(&self) -> usize {
        MESSAGE_LEN
    }

    /// Decodes into `out`, leaving it untouched on error.
    pub fn decode(
        &self,
        payload: &[u8],
        recv_ns: UnixNano,
        out: &mut WireRecord,
    ) -> Result<(), KrxError> {
        crate::message::validate(payload, MESSAGE_LEN)?;
        let msg = header(payload)?;

        let price = extract::derivative_price(msg.trcode, &msg.isin);
        let trade = fill_trade(payload, price)?;

        let mut h = RecordHeader::new(WireKind::Trade, Venue::Krx, crate::field::wire_symbol(&msg.isin), recv_ns);
        fill_record_header(&mut h, &msg, recv_ns, price.scale().unwrap_or_default());

        // No book in this message, so no depth and no emptiness to report. The
        // consumer must not read an absent book as an empty one: `depth` stays
        // zero and neither BID_EMPTY nor ASK_EMPTY is set, which is the "this
        // record carries no book" state rather than "both sides are void".

        *out = WireRecord::new_trade(h, trade);
        Ok(())
    }
}

/// `true` if this trcode is an `A3` on a derivative channel.
///
/// Unlike `B6`/`G7` there is nothing to choose: every derivative product group
/// sends the same 173-byte message, deep book or not.
pub const fn handles(trcode: TrCode) -> bool {
    trcode.is_derivative() && matches!(trcode.data_class(), [b'A', b'3'])
}
