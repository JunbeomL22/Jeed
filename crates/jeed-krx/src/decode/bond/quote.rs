//! `B6` — 일반채권·국고채권 우선호가. `IFMSRPD0023`, 462 B.
//!
//! ```text
//! [0:41]     header (shape B — no 정보분배종목인덱스)
//! [41:431]   5 × 78 B level block — 가격11×2 잔량15×2 수익률13×2
//! [431:446]  채권매도호가총잔량  ← no wire slot
//! [446:461]  채권매수호가총잔량  ← no wire slot
//! [461:462]  0xFF
//! ```

use crate::decode::bond::{self, DEPTH, LEVEL_LEN};
use crate::decode::common::fill_record_header;
use crate::error::KrxError;
use crate::extract::KRX;
use crate::trcode::TrCode;
use jeed_wire::{QuotePayload, RecordHeader, UnixNano, Venue, WireKind, WireRecord};

/// `IFMSRPD0023`.
pub const MESSAGE_LEN: usize = bond::HEADER_LEN + DEPTH * LEVEL_LEN + bond::BOOK_TAIL_LEN;

const _: () = assert!(MESSAGE_LEN == 462);

/// The 채권 우선호가 decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BondQuote;

/// `IFMSRPD0023`.
pub const DECODER: BondQuote = BondQuote;

impl BondQuote {
    /// Fixed message length this interface defines.
    #[inline]
    pub const fn message_len(&self) -> usize {
        MESSAGE_LEN
    }

    /// Levels per side this interface carries.
    #[inline]
    pub const fn depth(&self) -> usize {
        DEPTH
    }

    /// Decodes into `out`, leaving it untouched on error.
    pub fn decode(
        &self,
        payload: &[u8],
        recv_ns: UnixNano,
        out: &mut WireRecord,
    ) -> Result<(), KrxError> {
        crate::message::validate(payload, MESSAGE_LEN)?;
        let msg = bond::header(payload)?;

        let mut quote = QuotePayload::default();
        let shape = bond::fill_book(payload, bond::HEADER_LEN, &mut quote)?;

        let mut h = RecordHeader::new(WireKind::Quote, Venue::Krx, msg.isin, recv_ns);
        fill_record_header(&mut h, &msg, recv_ns, KRX.bond_price.scale().unwrap_or_default());
        h.set_depth(shape.depth).set_flags(shape.flags);

        *out = WireRecord::new_quote(h, quote);
        Ok(())
    }
}

/// `true` if this trcode is a `B6` on a 일반채권·국고채권 channel.
pub const fn handles(trcode: TrCode) -> bool {
    matches!(trcode.data_class(), [b'B', b'6']) && bond::is_bond_group(trcode)
}
