//! `A3` — 증권 체결. `IFMSRPD0004`, 186 B. 주식과 ETF·ELW·ETN이 같은 전문이다.
//!
//! ```text
//! [0:47]     header
//! [47:48]    전일대비구분코드      ← no wire slot (derivable from the tape)
//! [48:59]    전일대비가격          ← no wire slot
//! [59:70]    체결가격              → TradePayload::price
//! [70:80]    거래량                → TradePayload::qty
//! [80:91]    시가                  ← no wire slot
//! [91:102]   고가                  ← no wire slot
//! [102:113]  저가                  ← no wire slot
//! [113:125]  누적거래량            → TradePayload::cumulative_qty
//! [125:147]  누적거래대금          ← no wire slot
//! [147:148]  최종매도매수구분코드  → TradePayload::trade_kind
//! [148:163]  LP보유수량            ← no wire slot (ETN only, may be negative)
//! [163:174]  매도최우선호가가격    ← no wire slot
//! [174:185]  매수최우선호가가격    ← no wire slot
//! [185:186]  0xFF
//! ```
//!
//! ## The best bid and offer in here are deliberately dropped
//!
//! Unlike its derivative cousin this message ends with the top of book — but
//! **prices only**, no sizes. There is no wire shape for a book level whose
//! size is unknown: writing the prices with `qty = 0` would say "nothing rests
//! there", which is a claim about the market this message never made. Anything
//! taking 증권 체결 is taking `B6`/`B7` as well, where the real book is, so the
//! two fields are dropped rather than half-carried.
//!
//! ## No dynamic band
//!
//! 실시간가격제한 is a derivatives regime, so there is nothing here to fill
//! `dyn_upper`/`dyn_lower` with and `DYN_LIMIT_VALID` stays clear.

use crate::decode::common::{fill_record_header, header, slice};
use crate::decode::securities::{PRICE_LEN, QTY_LEN};
use crate::error::KrxError;
use crate::extract::KRX;
use crate::field;
use crate::trcode::TrCode;
use jeed_wire::{RecordHeader, TradePayload, UnixNano, Venue, WireKind, WireRecord, trade_kind};

// Offsets from documents/krx/layouts.md.
const OFF_PRICE: usize = 59;
const OFF_QTY: usize = 70;
const OFF_CUMULATIVE_QTY: usize = 113;
const OFF_AGGRESSOR: usize = 147;

/// `IFMSRPD0004`.
pub const MESSAGE_LEN: usize = 186;

/// The 증권 체결 decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SecuritiesTrade;

/// `IFMSRPD0004`.
pub const DECODER: SecuritiesTrade = SecuritiesTrade;

impl SecuritiesTrade {
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

        let reader = &KRX.securities_price;
        let price = field::price(reader, slice(payload, OFF_PRICE, PRICE_LEN), OFF_PRICE)?;
        let qty = field::uint(slice(payload, OFF_QTY, 10), OFF_QTY)?;
        let cumulative =
            field::uint(slice(payload, OFF_CUMULATIVE_QTY, QTY_LEN), OFF_CUMULATIVE_QTY)?;

        let mut trade = TradePayload::new(price.unwrap_or(0), qty.unwrap_or(0));
        if let Some(c) = cumulative {
            trade.with_cumulative_qty(c);
        }

        // 최종매도매수구분코드 is a space for every 단일가 cross, where there
        // is no aggressor by definition. UNKNOWN says "the field was there and
        // said nothing"; NONE would say the channel has no such field.
        trade.with_kind(match payload[OFF_AGGRESSOR] {
            b'1' => trade_kind::SELL,
            b'2' => trade_kind::BUY,
            _ => trade_kind::UNKNOWN,
        });

        let mut h = RecordHeader::new(WireKind::Trade, Venue::Krx, msg.isin, recv_ns);
        fill_record_header(&mut h, &msg, recv_ns, reader.scale().unwrap_or_default());

        *out = WireRecord::new_trade(h, trade);
        Ok(())
    }
}

/// `true` if this trcode is an `A3` on a 증권 channel.
///
/// 주식 (`01S`/`01Q`/`01X`) and the LP products (`02S`–`05S`) alike: one
/// interface covers the whole market.
pub const fn handles(trcode: TrCode) -> bool {
    matches!(trcode.data_class(), [b'A', b'3']) && is_securities_group(trcode)
}

/// `true` for the product groups 증권 real-time messages use.
pub(crate) const fn is_securities_group(trcode: TrCode) -> bool {
    matches!(
        trcode.product_group(),
        [b'0', b'1', b'S']
            | [b'0', b'1', b'Q']
            | [b'0', b'1', b'X']
            | [b'0', b'2', b'S']
            | [b'0', b'3', b'S']
            | [b'0', b'4', b'S']
            | [b'0', b'5', b'S']
    )
}
