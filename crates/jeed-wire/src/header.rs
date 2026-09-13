//! Fixed 64-byte record header.

use crate::error::WireError;
use crate::kind::{WireKind, header_flags};
use crate::types::{Scale, Symbol, UnixNano, Venue};
use crate::{WIRE_HEADER_LEN, WIRE_MAX_DEPTH};
use core::mem::size_of;

/// Record header — identical for every [`WireKind`].
///
/// Identity is `(venue, symbol)` in raw form: the consumer resolves it through
/// its own alias map using `recv_ns` as the resolve time
/// (`documents/feed_handler.md` §6). Ordering authority is `producer_seq`;
/// `recv_ns` is for measurement and labelling only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct RecordHeader {
    /// [`WireKind`] as a byte. `0` is invalid.
    pub kind: u8,

    /// [`Venue`] as a byte.
    pub venue: u8,

    /// [`header_flags`] bits.
    pub flags: u8,

    /// Number of valid book levels per side (≤ [`WIRE_MAX_DEPTH`]).
    /// Zero for kinds without a book.
    pub depth: u8,

    /// Price [`Scale`] as a byte. For [`WireKind::InvestorStats`] this is
    /// the *value* scale.
    pub price_scale: u8,

    /// Quantity [`Scale`] as a byte.
    pub qty_scale: u8,

    /// Explicit padding — always zero.
    pub _pad0: u16,

    /// Producer's monotonically increasing ring sequence (+1 per record
    /// written). A gap means the ring dropped records (§7).
    ///
    /// This is **not** the exchange's distribution sequence, which stays
    /// inside the handler — among other reasons because KRX leaves it blank
    /// on some channels (see `CLAUDE.md`).
    pub producer_seq: u64,

    /// Feed handler reception time. Becomes the consumer's `system_time`
    /// and the alias-resolve time.
    pub recv_ns: UnixNano,

    /// Assembled absolute exchange time in ns. Only meaningful when
    /// [`header_flags::VENUE_TIME_VALID`] is set. Midnight wrap of
    /// `HHMMSSuuuuuu` sources is resolved inside the handler.
    pub venue_ns: UnixNano,

    /// The venue's own instrument name, `NUL`-padded ([`Symbol`]).
    pub symbol: Symbol,

    /// Explicit padding — always zero.
    pub _pad1: [u8; 8],
}

const _: () = assert!(size_of::<RecordHeader>() == WIRE_HEADER_LEN);
const _: () = assert!(align_of::<RecordHeader>() == 8);
// No implicit padding: the field widths sum to the struct size.
const _: () = assert!(
    6 * size_of::<u8>()
        + size_of::<u16>()
        + size_of::<u64>()
        + 2 * size_of::<UnixNano>()
        + size_of::<Symbol>()
        + 8
        == size_of::<RecordHeader>()
);

impl RecordHeader {
    /// All-zero header. `kind == 0`, so it fails [`validate`](Self::validate)
    /// until a kind is assigned.
    pub const ZEROED: Self = Self {
        kind: 0,
        venue: 0,
        flags: 0,
        depth: 0,
        price_scale: 0,
        qty_scale: 0,
        _pad0: 0,
        producer_seq: 0,
        recv_ns: 0,
        venue_ns: 0,
        symbol: [0; crate::SYMBOL_LEN],
        _pad1: [0; 8],
    };

    /// Creates a header with the identity and reception time filled in.
    /// Scales default to `S0`; set them with [`set_scales`](Self::set_scales).
    #[inline]
    pub const fn new(kind: WireKind, venue: Venue, symbol: Symbol, recv_ns: UnixNano) -> Self {
        Self {
            kind: kind.as_u8(),
            venue: venue.as_u8(),
            symbol,
            recv_ns,
            ..Self::ZEROED
        }
    }

    /// The symbol without its `NUL` padding.
    #[inline]
    pub fn symbol_bytes(&self) -> &[u8] {
        crate::types::symbol_bytes(&self.symbol)
    }

    /// Decoded record kind.
    #[inline]
    pub const fn kind(&self) -> Result<WireKind, WireError> {
        WireKind::from_u8(self.kind)
    }

    /// Decoded venue.
    #[inline]
    pub const fn venue(&self) -> Result<Venue, WireError> {
        match Venue::from_u8(self.venue) {
            Some(v) => Ok(v),
            None => Err(WireError::Venue { found: self.venue }),
        }
    }

    /// Decoded price scale.
    #[inline]
    pub const fn price_scale(&self) -> Result<Scale, WireError> {
        scale_from_u8(self.price_scale)
    }

    /// Decoded quantity scale.
    #[inline]
    pub const fn qty_scale(&self) -> Result<Scale, WireError> {
        scale_from_u8(self.qty_scale)
    }

    /// Sets both scales.
    #[inline]
    pub const fn set_scales(&mut self, price: Scale, qty: Scale) -> &mut Self {
        self.price_scale = price.decimals();
        self.qty_scale = qty.decimals();
        self
    }

    /// Sets the number of valid levels per side.
    #[inline]
    pub const fn set_depth(&mut self, depth: u8) -> &mut Self {
        self.depth = depth;
        self
    }

    /// Sets the ring sequence.
    #[inline]
    pub const fn set_producer_seq(&mut self, seq: u64) -> &mut Self {
        self.producer_seq = seq;
        self
    }

    /// Sets the exchange time and marks it valid.
    #[inline]
    pub const fn set_venue_time(&mut self, venue_ns: UnixNano) -> &mut Self {
        self.venue_ns = venue_ns;
        self.flags |= header_flags::VENUE_TIME_VALID;
        self
    }

    /// Exchange time, if the producer marked it valid.
    #[inline]
    pub const fn venue_time(&self) -> Option<UnixNano> {
        if self.has(header_flags::VENUE_TIME_VALID) {
            Some(self.venue_ns)
        } else {
            None
        }
    }

    /// `true` if every bit of `mask` is set.
    #[inline]
    pub const fn has(&self, mask: u8) -> bool {
        self.flags & mask == mask
    }

    /// Sets the given flag bits.
    #[inline]
    pub const fn set_flags(&mut self, mask: u8) -> &mut Self {
        self.flags |= mask;
        self
    }

    /// Producer marked this record stale.
    #[inline]
    pub const fn is_stale(&self) -> bool {
        self.has(header_flags::STALE)
    }

    /// Venue-to-reception age in ns when the venue time is valid.
    ///
    /// Saturates at zero: neither clock is monotonic and `UnixNano` is
    /// unsigned, so a plain subtraction would wrap into a huge age (§6).
    #[inline]
    pub const fn venue_age_ns(&self) -> Option<UnixNano> {
        match self.venue_time() {
            Some(v) => Some(self.recv_ns.saturating_sub(v)),
            None => None,
        }
    }

    /// Checks every enumerated byte and the depth bound.
    pub const fn validate(&self) -> Result<(), WireError> {
        if let Err(e) = self.kind() {
            return Err(e);
        }
        if let Err(e) = self.venue() {
            return Err(e);
        }
        if let Err(e) = self.price_scale() {
            return Err(e);
        }
        if let Err(e) = self.qty_scale() {
            return Err(e);
        }
        if self.depth as usize > WIRE_MAX_DEPTH {
            return Err(WireError::Depth {
                found: self.depth,
                max: WIRE_MAX_DEPTH as u8,
            });
        }
        Ok(())
    }
}

/// Decodes a scale byte (`Scale` uses its decimal count as discriminant).
#[inline]
pub(crate) const fn scale_from_u8(v: u8) -> Result<Scale, WireError> {
    match Scale::from_decimals(v as usize) {
        Some(s) => Ok(s),
        None => Err(WireError::Scale { found: v }),
    }
}
