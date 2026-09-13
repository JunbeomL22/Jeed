//! What a crypto decoder is pinned to.
//!
//! ## Why the decoders are per-instrument and the KRX ones are not
//!
//! A KRX datagram says which instrument it is about — the ISIN is at a known
//! offset, and one socket carries every instrument in a product group, so the
//! receive loop demultiplexes (`documents/decoder-pattern.md`). A crypto
//! subscription is the other way round: you connect to
//! `wss://…/ws/btcusdt@trade` and every frame on it is BTCUSDT, so the
//! exchange has already demultiplexed and the decoder is told once.
//!
//! That is also where the scales come from. KRX settles a price's decimals
//! from the product group and the ISIN prefix, because the standard says so.
//! No such rule exists for crypto: `tickSize` and `stepSize` are per symbol,
//! published in `exchangeInfo`, and they change. So they are configuration,
//! carried here, resolved before the first frame — and stamped on the record
//! header, never travelling beside a value (`jeed_convert`'s crate docs).

use crate::error::{CryptoError, InstrumentError};
use crate::json::{PLAIN_DECIMAL_LEN, plain_decimal};
use jeed_convert::DynamicExtractor;
use jeed_wire::{
    BookPrice, BookQuantity, RecordHeader, Scale, Symbol, UnixNano, Venue, WireKind,
    symbol_bytes, symbol_from_bytes,
};

/// One symbol on one venue, with the scales its prices and sizes are read on.
///
/// Built once at start-up and shared by every stream of that instrument: the
/// trade decoder and the book decoder read the same numbers the same way, and
/// there is one place a wrong `exchangeInfo` precision can be wrong.
#[derive(Debug, Clone, PartialEq)]
pub struct Instrument {
    venue: Venue,
    symbol: Symbol,
    symbol_len: usize,
    price: DynamicExtractor,
    qty: DynamicExtractor,
    price_scale: Scale,
    qty_scale: Scale,
}

impl Instrument {
    /// Pins a decoder to `symbol` on `venue`, reading prices at
    /// `price_decimals` and sizes at `qty_decimals`.
    ///
    /// `symbol` is the venue's own spelling in the venue's own casing —
    /// `BTCUSDT`, not `BTC_USDT`. Normalising a pair name is a mapping, and a
    /// mapping is the consumer's (`documents/feed_handler.md` §6); what
    /// crosses the boundary is what the venue said.
    pub fn new(
        venue: Venue,
        symbol: &[u8],
        price_decimals: usize,
        qty_decimals: usize,
    ) -> Result<Self, CryptoError> {
        let s = symbol_from_bytes(symbol).ok_or(InstrumentError::Symbol)?;
        let price_scale =
            Scale::from_decimals(price_decimals).ok_or(InstrumentError::Scale { decimals: price_decimals })?;
        let qty_scale =
            Scale::from_decimals(qty_decimals).ok_or(InstrumentError::Scale { decimals: qty_decimals })?;

        Ok(Self {
            venue,
            symbol: s,
            symbol_len: symbol.len(),
            price: DynamicExtractor::new(price_decimals),
            qty: DynamicExtractor::new(qty_decimals),
            price_scale,
            qty_scale,
        })
    }

    /// The venue.
    #[inline]
    pub const fn venue(&self) -> Venue {
        self.venue
    }

    /// The wire symbol field, `NUL`-padded.
    #[inline]
    pub const fn symbol(&self) -> &Symbol {
        &self.symbol
    }

    /// The venue's symbol without its padding.
    #[inline]
    pub fn symbol_bytes(&self) -> &[u8] {
        symbol_bytes(&self.symbol)
    }

    /// Price scale.
    #[inline]
    pub const fn price_scale(&self) -> Scale {
        self.price_scale
    }

    /// Quantity scale.
    #[inline]
    pub const fn qty_scale(&self) -> Scale {
        self.qty_scale
    }

    /// Reads a decimal-string price on this instrument's scale.
    ///
    /// `to_i64_exact`, not `to_i64`: the venue chooses how many decimals it
    /// sends, so a truncating read would turn a real price into a smaller
    /// real-looking price. Binance pads to full precision, so the digits past
    /// the scale are normally zeros and this costs one comparison
    /// (`jeed_convert::DynamicExtractor::to_i64_exact`).
    ///
    /// A number in exponent form (`1.04525E8` — Upbit and Bithumb above ten
    /// million) is rewritten plain first, digit for digit
    /// ([`plain_decimal`]). Every other venue's numbers pass through
    /// untouched; the check for an `E` is one scan of a short field.
    #[inline]
    pub fn price(&self, bytes: &[u8], key: &'static str) -> Result<BookPrice, CryptoError> {
        let mut plain = [0u8; PLAIN_DECIMAL_LEN];
        let bytes = match plain_decimal(bytes, &mut plain) {
            Some(n) => &plain[..n],
            None => bytes,
        };
        self.price.to_i64_exact(bytes).map_err(|err| CryptoError::Field { key, err })
    }

    /// Reads a decimal-string size on this instrument's scale. Exponent form
    /// is handled as in [`price`](Self::price).
    #[inline]
    pub fn qty(&self, bytes: &[u8], key: &'static str) -> Result<BookQuantity, CryptoError> {
        let mut plain = [0u8; PLAIN_DECIMAL_LEN];
        let bytes = match plain_decimal(bytes, &mut plain) {
            Some(n) => &plain[..n],
            None => bytes,
        };
        self.qty.to_u64_exact(bytes).map_err(|err| CryptoError::Field { key, err })
    }

    /// `true` when the frame's `s` is this instrument.
    ///
    /// Compared against the venue's spelling, so it is byte-for-byte and
    /// case-sensitive: Binance sends `s` upper-case even though the stream URL
    /// is lower-case, and a decoder configured with the URL casing should
    /// find out loudly rather than accept everything.
    #[inline]
    pub fn matches(&self, s: &[u8]) -> bool {
        s.len() == self.symbol_len && s == self.symbol_bytes()
    }

    /// Checks a frame's `s` against this instrument.
    #[inline]
    pub fn check_symbol(&self, s: &[u8]) -> Result<(), CryptoError> {
        if self.matches(s) { Ok(()) } else { Err(CryptoError::SymbolMismatch) }
    }

    /// A record header for this instrument, with both scales already set.
    #[inline]
    pub fn header(&self, kind: WireKind, recv_ns: UnixNano) -> RecordHeader {
        let mut h = RecordHeader::new(kind, self.venue, self.symbol, recv_ns);
        h.set_scales(self.price_scale, self.qty_scale);
        h
    }
}
