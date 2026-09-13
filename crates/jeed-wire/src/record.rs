//! The fixed-size wire record and its byte views.

use crate::error::WireError;
use crate::header::RecordHeader;
use crate::kind::{
    WireKind, dyn_limit_action, expansion_direction, level_ext, quote_ext, trade_kind,
};
use crate::payload::{
    HeartbeatPayload, InvestorStatsPayload, MarketSchedulePayload, OpenInterestPayload,
    DynamicPriceLimitPayload, PriceLimitPayload, QuotePayload, TradePayload, TradeQuotePayload,
    WirePayload,
};
use crate::{WIRE_ALIGN, WIRE_HEADER_LEN, WIRE_PAYLOAD_LEN, WIRE_RECORD_LEN};
use core::fmt;

/// Explicit tail padding so the record is a whole number of cache lines.
const TAIL_LEN: usize = WIRE_RECORD_LEN - WIRE_HEADER_LEN - WIRE_PAYLOAD_LEN;

/// One ring slot: header + payload (+ tail padding when the sum is not a whole
/// number of cache lines; zero-length today), cache-line aligned.
///
/// A record is always fully initialised (construct via [`zeroed`](Self::zeroed)
/// or the `new_*` constructors), which is what makes [`as_bytes`](Self::as_bytes)
/// sound. Typed payload access is gated by the header kind; a mismatch is an
/// error, never a reinterpretation.
#[derive(Clone, Copy)]
#[repr(C, align(64))]
pub struct WireRecord {
    /// Fixed header.
    pub header: RecordHeader,

    payload: WirePayload,

    _tail: [u8; TAIL_LEN],
}

const _: () = assert!(size_of::<WireRecord>() == WIRE_RECORD_LEN);
const _: () = assert!(align_of::<WireRecord>() == WIRE_ALIGN);
const _: () = assert!(WIRE_RECORD_LEN.is_multiple_of(WIRE_ALIGN));
const _: () = assert!(core::mem::offset_of!(WireRecord, header) == 0);
const _: () = assert!(core::mem::offset_of!(WireRecord, payload) == WIRE_HEADER_LEN);
const _: () =
    assert!(core::mem::offset_of!(WireRecord, _tail) == WIRE_HEADER_LEN + WIRE_PAYLOAD_LEN);

macro_rules! typed_access {
    ($kind:ident, $member:ident, $ty:ty, $get:ident, $get_mut:ident, $set:ident, $new:ident) => {
        #[doc = concat!("Payload view for [`WireKind::", stringify!($kind), "`].")]
        #[inline]
        pub fn $get(&self) -> Result<&$ty, WireError> {
            self.expect_kind(WireKind::$kind)?;
            // SAFETY: every union member is plain-old-data whose every bit
            // pattern is valid, and the union is always fully initialised.
            Ok(unsafe { &self.payload.$member })
        }

        #[doc = concat!("Mutable payload view for [`WireKind::", stringify!($kind), "`].")]
        #[inline]
        pub fn $get_mut(&mut self) -> Result<&mut $ty, WireError> {
            self.expect_kind(WireKind::$kind)?;
            // SAFETY: as above; writes through the reference keep the union
            // fully initialised because the member has no padding.
            Ok(unsafe { &mut self.payload.$member })
        }

        #[doc = concat!(
            "Installs a [`WireKind::", stringify!($kind),
            "`] payload, zeroing the rest of the payload area and setting the header kind."
        )]
        #[inline]
        pub fn $set(&mut self, payload: $ty) -> &mut Self {
            self.payload = WirePayload::ZEROED;
            self.payload.$member = payload;
            self.header.kind = WireKind::$kind.as_u8();
            self
        }

        #[doc = concat!("Builds a [`WireKind::", stringify!($kind), "`] record.")]
        #[inline]
        pub fn $new(header: RecordHeader, payload: $ty) -> Self {
            let mut rec = Self::with_header(header);
            rec.$set(payload);
            rec
        }
    };
}

impl WireRecord {
    /// All-zero record. Its kind byte is `0`, so it fails
    /// [`validate`](Self::validate) until a payload is installed.
    #[inline]
    pub const fn zeroed() -> Self {
        Self {
            header: RecordHeader::ZEROED,
            payload: WirePayload::ZEROED,
            _tail: [0; TAIL_LEN],
        }
    }

    /// Zero payload with the given header.
    #[inline]
    pub const fn with_header(header: RecordHeader) -> Self {
        Self {
            header,
            payload: WirePayload::ZEROED,
            _tail: [0; TAIL_LEN],
        }
    }

    typed_access!(Quote, quote, QuotePayload, quote, quote_mut, set_quote, new_quote);
    typed_access!(Trade, trade, TradePayload, trade, trade_mut, set_trade, new_trade);
    typed_access!(
        TradeQuote,
        trade_quote,
        TradeQuotePayload,
        trade_quote,
        trade_quote_mut,
        set_trade_quote,
        new_trade_quote
    );
    typed_access!(
        OpenInterest,
        open_interest,
        OpenInterestPayload,
        open_interest,
        open_interest_mut,
        set_open_interest,
        new_open_interest
    );
    typed_access!(
        InvestorStats,
        investor_stats,
        InvestorStatsPayload,
        investor_stats,
        investor_stats_mut,
        set_investor_stats,
        new_investor_stats
    );
    typed_access!(
        PriceLimit,
        price_limit,
        PriceLimitPayload,
        price_limit,
        price_limit_mut,
        set_price_limit,
        new_price_limit
    );
    typed_access!(
        MarketSchedule,
        market_schedule,
        MarketSchedulePayload,
        market_schedule,
        market_schedule_mut,
        set_market_schedule,
        new_market_schedule
    );
    typed_access!(
        DynamicPriceLimit,
        dynamic_price_limit,
        DynamicPriceLimitPayload,
        dynamic_price_limit,
        dynamic_price_limit_mut,
        set_dynamic_price_limit,
        new_dynamic_price_limit
    );
    typed_access!(
        Heartbeat,
        heartbeat,
        HeartbeatPayload,
        heartbeat,
        heartbeat_mut,
        set_heartbeat,
        new_heartbeat
    );

    /// Decoded record kind.
    #[inline]
    pub const fn kind(&self) -> Result<WireKind, WireError> {
        self.header.kind()
    }

    /// Raw payload bytes.
    #[inline]
    pub const fn payload_bytes(&self) -> &[u8; WIRE_PAYLOAD_LEN] {
        self.payload.bytes()
    }

    #[inline]
    fn expect_kind(&self, expected: WireKind) -> Result<(), WireError> {
        let found = self.kind()?;
        if found == expected {
            Ok(())
        } else {
            Err(WireError::KindMismatch { expected, found })
        }
    }

    /// Full validation: header enumerations, depth bound, and the
    /// kind-specific encodings a consumer will need to decode.
    pub fn validate(&self) -> Result<(), WireError> {
        self.header.validate()?;
        match self.kind()? {
            WireKind::Quote => validate_quote(self.quote()?),
            WireKind::Trade => validate_trade(self.trade()?),
            WireKind::TradeQuote => {
                let tq = self.trade_quote()?;
                validate_trade(&tq.trade)?;
                validate_quote(&tq.quote)
            }
            WireKind::MarketSchedule => {
                let ms = self.market_schedule()?;
                match ms.expansion_direction {
                    expansion_direction::NOT_APPLICABLE
                    | expansion_direction::UP
                    | expansion_direction::DOWN => Ok(()),
                    found => Err(WireError::ExpansionDirection { found }),
                }
            }
            WireKind::DynamicPriceLimit => {
                let d = self.dynamic_price_limit()?;
                match d.action {
                    dyn_limit_action::UNKNOWN
                    | dyn_limit_action::APPLIED
                    | dyn_limit_action::RELEASED => Ok(()),
                    found => Err(WireError::DynLimitAction { found }),
                }
            }
            WireKind::OpenInterest
            | WireKind::InvestorStats
            | WireKind::SnapshotDelta
            | WireKind::PriceLimit
            | WireKind::Heartbeat => Ok(()),
        }
    }

    // ------------------------------------------------------------------
    // Byte views
    // ------------------------------------------------------------------

    /// The record as bytes (its exact in-memory image).
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; WIRE_RECORD_LEN] {
        // SAFETY: `WireRecord` is `repr(C)` with no implicit padding (all
        // padding is explicit zeroed fields, asserted at compile time) and is
        // only ever constructed fully initialised, so every byte is
        // initialised. Size equality is asserted at compile time.
        unsafe { &*(self as *const Self as *const [u8; WIRE_RECORD_LEN]) }
    }

    /// Copies a record out of a byte slice and validates it.
    /// The slice need not be aligned.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() != WIRE_RECORD_LEN {
            return Err(WireError::Length {
                expected: WIRE_RECORD_LEN,
                actual: bytes.len(),
            });
        }
        // SAFETY: length checked above; every bit pattern is a valid
        // `WireRecord` because all fields are plain-old-data; the read is
        // unaligned-tolerant.
        let rec: Self = unsafe { core::ptr::read_unaligned(bytes.as_ptr() as *const Self) };
        rec.validate()?;
        Ok(rec)
    }

    /// Borrows a record in place from an aligned byte slice and validates it.
    pub fn ref_from_bytes(bytes: &[u8]) -> Result<&Self, WireError> {
        if bytes.len() != WIRE_RECORD_LEN {
            return Err(WireError::Length {
                expected: WIRE_RECORD_LEN,
                actual: bytes.len(),
            });
        }
        if !(bytes.as_ptr() as usize).is_multiple_of(WIRE_ALIGN) {
            return Err(WireError::Alignment { required: WIRE_ALIGN });
        }
        // SAFETY: length and alignment checked; all bit patterns valid.
        let rec: &Self = unsafe { &*(bytes.as_ptr() as *const Self) };
        rec.validate()?;
        Ok(rec)
    }
}

/// Checks the enumerated bytes of a quote payload.
fn validate_quote(q: &QuotePayload) -> Result<(), WireError> {
    match q.level_ext_kind {
        level_ext::NONE | level_ext::BOND_YIELD | level_ext::LP_QUANTITY => {}
        found => return Err(WireError::LevelExtKind { found }),
    }
    match q.quote_ext_kind {
        quote_ext::NONE | quote_ext::LP_HOLDINGS | quote_ext::SEQUENCE | quote_ext::EXPECTED_PRICE => Ok(()),
        found => Err(WireError::QuoteExtKind { found }),
    }
}

/// Checks the enumerated bytes of a trade payload.
fn validate_trade(t: &TradePayload) -> Result<(), WireError> {
    match t.trade_kind {
        trade_kind::NONE | trade_kind::SELL | trade_kind::BUY | trade_kind::UNKNOWN => Ok(()),
        found => Err(WireError::TradeKind { found }),
    }
}

impl Default for WireRecord {
    #[inline]
    fn default() -> Self {
        Self::zeroed()
    }
}

impl PartialEq for WireRecord {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for WireRecord {}

impl fmt::Debug for WireRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("WireRecord");
        d.field("header", &self.header);
        match self.kind() {
            Ok(WireKind::Quote) => d.field("quote", &self.quote().ok()),
            Ok(WireKind::Trade) => d.field("trade", &self.trade().ok()),
            Ok(WireKind::TradeQuote) => d.field("trade_quote", &self.trade_quote().ok()),
            Ok(WireKind::OpenInterest) => d.field("open_interest", &self.open_interest().ok()),
            Ok(WireKind::InvestorStats) => d.field("investor_stats", &self.investor_stats().ok()),
            Ok(WireKind::PriceLimit) => d.field("price_limit", &self.price_limit().ok()),
            Ok(WireKind::DynamicPriceLimit) => {
                d.field("dynamic_price_limit", &self.dynamic_price_limit().ok())
            }
            Ok(WireKind::MarketSchedule) => d.field("market_schedule", &self.market_schedule().ok()),
            Ok(WireKind::SnapshotDelta) | Err(_) => d.field("payload", &self.payload),
            Ok(WireKind::Heartbeat) => d.field("heartbeat", &self.heartbeat().ok()),
        };
        d.finish()
    }
}
