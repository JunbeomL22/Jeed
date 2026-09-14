//! `B6` — 소액채권 우선호가. `IFMSRPD0024`, 882 B.
//!
//! ```text
//! [0:41]     header (shape B — no 정보분배종목인덱스)
//! [41:821]   5 × 156 B level block — 종목 78 B then 종류 78 B
//! [821:851]  채권매도/매수호가총잔량      ← no wire slot
//! [851:881]  채권종류매도/매수호가총잔량  ← no wire slot
//! [881:882]  0xFF
//! ```

use crate::decode::bond::{self, DEPTH};
use crate::decode::bond::small_lot::{BOOK_TAIL_LEN, LEVEL_LEN};
use crate::decode::common::fill_record_header;
use crate::error::KrxError;
use crate::extract::KRX;
use crate::trcode::TrCode;
use jeed_wire::{QuotePayload, RecordHeader, UnixNano, Venue, WireKind, WireRecord};

/// `IFMSRPD0024`.
pub const MESSAGE_LEN: usize = bond::HEADER_LEN + DEPTH * LEVEL_LEN + BOOK_TAIL_LEN;

const _: () = assert!(MESSAGE_LEN == 882);

/// The 소액채권 우선호가 decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SmallLotQuote;

/// `IFMSRPD0024`.
pub const DECODER: SmallLotQuote = SmallLotQuote;

impl SmallLotQuote {
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
        let shape = bond::fill_book(payload, bond::HEADER_LEN, LEVEL_LEN, &mut quote)?;

        let mut h = RecordHeader::new(WireKind::Quote, Venue::Krx, crate::field::wire_symbol(&msg.isin), recv_ns);
        fill_record_header(&mut h, &msg, recv_ns, KRX.bond_price.scale().unwrap_or_default());
        h.set_depth(shape.depth).set_flags(shape.flags);

        *out = WireRecord::new_quote(h, quote);
        Ok(())
    }
}

/// `true` if this trcode is a `B6` on the 소액채권 channel.
pub const fn handles(trcode: TrCode) -> bool {
    matches!(trcode.data_class(), [b'B', b'6']) && bond::is_small_lot_group(trcode)
}
