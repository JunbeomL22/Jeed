//! `B6` — 파생 우선호가. `IFMSRPD0034` (5-deep, 324 B) and `IFMSRPD0035`
//! (10-deep, 554 B).
//!
//! ```text
//! [0:47]    header (documents/krx/layouts.md)
//! [47:..]   depth × 46 B level block
//! [..+0:9]  매도호가총잔량      ← no wire slot (todo.md §10)
//! [..+9:18] 매수호가총잔량      ← no wire slot
//! [..+18:23] 매도호가유효건수   ← no wire slot
//! [..+23:28] 매수호가유효건수   ← no wire slot
//! [..+28:37] 예상체결가         → quote_ext
//! [..+37:46] 예상체결수량       ← no wire slot
//! [..+46:47] 0xFF
//! ```

use crate::decode::derivative::{HEADER_LEN, LEVEL_LEN, fill_book, fill_record_header, header, slice};
use crate::error::KrxError;
use crate::field;
use crate::trcode::TrCode;
use jeed_wire::{
    QuotePayload, RecordHeader, UnixNano, Venue, WireKind, WireRecord, quote_ext,
};

/// Offset of 예상체결가 relative to the end of the level blocks.
const TAIL_EXPECTED_PRICE: usize = 28;

/// Bytes after the last level block.
const TAIL_LEN: usize = 47;

/// `IFMSRPD0034` — five levels per side. Index derivatives, bond futures,
/// currency futures.
pub const FIVE_DEEP: DerivativeQuote = DerivativeQuote::new(5);

/// `IFMSRPD0035` — ten levels per side. Single-stock futures and options
/// (`B604F`, `B605F`).
pub const TEN_DEEP: DerivativeQuote = DerivativeQuote::new(10);

/// A `B6` decoder for one book depth.
///
/// The two depths differ only in how many level blocks sit between the header
/// and the tail, so they are one decoder parameterised rather than two
/// near-copies that drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DerivativeQuote {
    depth: usize,
    message_len: usize,
}

impl DerivativeQuote {
    /// Decoder for `depth` levels per side.
    pub const fn new(depth: usize) -> Self {
        Self { depth, message_len: HEADER_LEN + depth * LEVEL_LEN + TAIL_LEN }
    }

    /// Levels per side this variant carries.
    #[inline]
    pub const fn depth(&self) -> usize {
        self.depth
    }

    /// Fixed message length this variant expects.
    #[inline]
    pub const fn message_len(&self) -> usize {
        self.message_len
    }

    /// Decodes into `out`.
    ///
    /// `out` is left untouched on error: the record is built on the stack and
    /// installed only once every field has parsed, so a failure cannot leave a
    /// half-filled record for the ring to publish.
    pub fn decode(
        &self,
        payload: &[u8],
        recv_ns: UnixNano,
        out: &mut WireRecord,
    ) -> Result<(), KrxError> {
        crate::message::validate(payload, self.message_len)?;
        let msg = header(payload)?;

        let mut quote = QuotePayload::default();
        let shape = fill_book(payload, HEADER_LEN, self.depth, &mut quote)?;

        // 예상체결가 — the indicative price during a call auction, before the
        // cross. It is the one tail field with somewhere to go: `quote_ext` is
        // a spare u64 that derivative channels otherwise leave empty. The four
        // aggregate fields around it (총잔량 ×2, 유효건수 ×2) have no slot at
        // all; see todo.md §10.
        let tail = HEADER_LEN + self.depth * LEVEL_LEN;
        let expected = field::decimal(
            slice(payload, tail + TAIL_EXPECTED_PRICE, 9),
            tail + TAIL_EXPECTED_PRICE,
        )?;
        if let Some(d) = expected
            && d.value != 0
        {
            quote.with_quote_ext(quote_ext::EXPECTED_PRICE, d.value as u64);
        }

        let mut h = RecordHeader::new(WireKind::Quote, Venue::Krx, msg.isin, recv_ns);
        fill_record_header(&mut h, &msg, recv_ns, shape.price_scale);
        h.set_depth(shape.depth).set_flags(shape.flags);

        *out = WireRecord::new_quote(h, quote);
        Ok(())
    }
}

/// The trcodes this decoder handles, by book depth.
///
/// Five-deep everywhere except single-stock futures (`04F`) and single-stock
/// options (`05F`), which are ten-deep.
pub const fn depth_for(trcode: TrCode) -> Option<usize> {
    if !trcode.is_derivative() {
        return None;
    }
    match trcode.data_class() {
        [b'B', b'6'] => match trcode.product_group() {
            [b'0', b'4', b'F'] | [b'0', b'5', b'F'] => Some(10),
            _ => Some(5),
        },
        _ => None,
    }
}
