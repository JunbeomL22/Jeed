//! A stand-in venue, and the tests that pin its shape.
//!
//! `jeed-fix` ships no adapter — FIX is a protocol and this crate knows no
//! venue (`documents/feed_handler.md` §13) — so the pipeline cannot be tested
//! without one. This is modelled on the real SMBS adapter in fractal-engine and
//! keeps two of its venue facts, because each is a shape the pipeline has to
//! cope with:
//!
//! - quotes carry no size, so one is supplied;
//! - a `35=X` that moves the **book** is refused, because this venue has no
//!   `SnapshotDelta` mapping and dropping it quietly would leave the
//!   consumer's book wrong.
//!
//! Its third one went away with wire v2. The identifier used to be a symbol
//! squeezed left-aligned into twelve ISIN bytes; the header now carries a
//! twenty-four-byte [`Symbol`](jeed_wire::Symbol) that *is* the venue's own
//! name for the instrument, so there is nothing to squeeze.
//!
//! It is not the SMBS adapter and does not try to be. When the real one lands
//! it belongs beside the venue, not here.

use jeed_fix::recv::MdAdapter;
use jeed_fix::{FixScales, MdEntryType, MdMessage};
use jeed_wire::{
    QuotePayload, RecordHeader, RecordSink, SYMBOL_LEN, Scale, Symbol, TradePayload, UnixNano,
    Venue, WireKind, WireLevel, WireRecord, symbol_from_bytes,
};

/// Why the stand-in venue refused a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FakeError {
    /// A `35=X` entry that moves the book. The wire has no delta record
    /// (`documents/feed_handler.md` §8 reserves `SnapshotDelta`), so this is
    /// refused rather than dropped.
    IncrementalBook,

    /// No `55` anywhere, so there is nothing to key the record on.
    NoSymbol,

    /// A `55` longer than the header's symbol field. Twenty-four bytes covers
    /// every venue symbol we know of, so this is a venue doing something new
    /// rather than a record to truncate into the wrong instrument.
    SymbolTooLong,
}

impl core::fmt::Display for FakeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::IncrementalBook => write!(f, "35=X moves the book and the wire has no delta"),
            Self::NoSymbol => write!(f, "no Symbol to key the record on"),
            Self::SymbolTooLong => write!(f, "Symbol is longer than {SYMBOL_LEN} bytes"),
        }
    }
}

/// A venue that quotes one FX pair two decimals wide.
#[derive(Debug, Clone, Copy)]
pub struct FakeVenue {
    /// How its text becomes integers.
    pub scales: FixScales,

    /// Size to publish when a quote carries none.
    pub default_size: u64,

    /// Refuse `35=X` book entries. False lets a test drive the happy path with
    /// incremental book updates a future wire format would carry.
    pub refuse_book_deltas: bool,
}

impl Default for FakeVenue {
    fn default() -> Self {
        Self {
            scales: FixScales::new(Scale::S2, Scale::S0),
            default_size: 5_000_000,
            refuse_book_deltas: true,
        }
    }
}

/// The venue's symbol as the header carries it.
///
/// Nothing venue-specific left in it: wire v2's `Symbol` is the venue's own
/// name for the instrument, so this is `symbol_from_bytes` and a choice about
/// what to do when it says no.
pub fn wire_symbol(symbol: &[u8]) -> Result<Symbol, FakeError> {
    if symbol.is_empty() {
        return Err(FakeError::NoSymbol);
    }
    symbol_from_bytes(symbol).ok_or(FakeError::SymbolTooLong)
}

impl MdAdapter for FakeVenue {
    type Error = FakeError;

    fn venue(&self) -> Venue {
        Venue::Smbs
    }

    fn scales(&self) -> FixScales {
        self.scales
    }

    fn adapt<S: RecordSink>(
        &mut self,
        msg: &MdMessage,
        recv_ns: UnixNano,
        sink: &mut S,
    ) -> Result<usize, FakeError> {
        if msg.is_snapshot() {
            return self.snapshot(msg, recv_ns, sink);
        }
        self.incremental(msg, recv_ns, sink)
    }
}

impl FakeVenue {
    /// `35=W` → one quote record.
    fn snapshot<S: RecordSink>(
        &self,
        msg: &MdMessage,
        recv_ns: UnixNano,
        sink: &mut S,
    ) -> Result<usize, FakeError> {
        let symbol = wire_symbol(msg.symbol.as_bytes())?;

        let mut quote = QuotePayload::default();
        let mut depth = 0usize;
        for entry in msg.entries() {
            let side = match entry.entry_type {
                MdEntryType::Bid | MdEntryType::Offer => entry.entry_type,
                // A statistic entry in a snapshot is not a book level; the
                // decoder carries it so a venue adding one cannot break us.
                _ => continue,
            };
            let level = entry.level as usize;
            if level >= jeed_wire::WIRE_MAX_DEPTH {
                continue;
            }
            let qty = if entry.has_size { entry.size } else { self.default_size };
            let wire = WireLevel::new(entry.price, qty);
            if matches!(side, MdEntryType::Bid) {
                quote.set_bid(level, wire);
            } else {
                quote.set_ask(level, wire);
            }
            depth = depth.max(level + 1);
        }

        let mut header = RecordHeader::new(WireKind::Quote, Venue::Smbs, symbol, recv_ns);
        header.set_scales(self.scales.price, self.scales.quantity);
        header.set_depth(depth as u8);
        if let Some(venue_ns) = msg.venue_time() {
            header.set_venue_time(venue_ns);
        }
        if msg.side_depth(MdEntryType::Bid) == 0 {
            header.set_flags(jeed_wire::header_flags::BID_EMPTY);
        }
        if msg.side_depth(MdEntryType::Offer) == 0 {
            header.set_flags(jeed_wire::header_flags::ASK_EMPTY);
        }

        sink.publish(|rec| {
            *rec = WireRecord::new_quote(header, quote);
            Ok::<(), FakeError>(())
        })?;
        Ok(1)
    }

    /// `35=X` → one trade record per print, and a refusal for anything that
    /// would move the book.
    fn incremental<S: RecordSink>(
        &self,
        msg: &MdMessage,
        recv_ns: UnixNano,
        sink: &mut S,
    ) -> Result<usize, FakeError> {
        let mut published = 0;
        for entry in msg.entries() {
            match entry.entry_type {
                MdEntryType::Trade => {}
                MdEntryType::Bid | MdEntryType::Offer if self.refuse_book_deltas => {
                    return Err(FakeError::IncrementalBook);
                }
                _ => continue,
            }

            let symbol = wire_symbol(msg.entry_symbol(entry))?;
            let qty = if entry.has_size { entry.size } else { self.default_size };
            let payload = TradePayload::new(entry.price, qty);

            let mut header = RecordHeader::new(WireKind::Trade, Venue::Smbs, symbol, recv_ns);
            header.set_scales(self.scales.price, self.scales.quantity);
            if let Some(venue_ns) = entry.venue_time() {
                header.set_venue_time(venue_ns);
            }

            sink.publish(|rec| {
                *rec = WireRecord::new_trade(header, payload);
                Ok::<(), FakeError>(())
            })?;
            published += 1;
        }
        Ok(published)
    }
}

#[test]
fn a_symbol_rides_the_header_field_nul_padded() {
    let s = wire_symbol(b"USDKRW").expect("fits");
    assert_eq!(&s[..6], b"USDKRW");
    assert_eq!(&s[6..], &[0u8; SYMBOL_LEN - 6], "NUL, so an all-zero slot stays unambiguous");
    assert_eq!(jeed_wire::symbol_bytes(&s), b"USDKRW");

    // Long enough for an OKX option (22) and then some.
    assert!(wire_symbol(b"BTC-USD-240329-70000-C").is_ok());
    assert_eq!(wire_symbol(&[b'X'; SYMBOL_LEN + 1]), Err(FakeError::SymbolTooLong));
    assert_eq!(wire_symbol(b""), Err(FakeError::NoSymbol));
}
