//! `35=W` / `35=X` market-data decoding into a stack-allocated message.
//!
//! This module knows the standard FIX 4.4 market-data tags and nothing about
//! any venue: no ISIN mapping, no synthetic sizes, no wire records. The
//! output ([`MdMessage`]) is a fixed-capacity value the caller turns into
//! whatever it needs — a venue adapter builds
//! [`WireRecord`](jeed_wire::WireRecord)s from it.
//!
//! Two decisions worth stating:
//!
//! - **Prices and sizes are integers here already.** The decimal text is
//!   scaled once ([`FixScales`]) at the point of parse, exactly as the KRX
//!   parquet reader does, so the rounding point never moves downstream. A
//!   value with more fractional digits than the scale keeps is an error
//!   ([`FixError::Precision`]) rather than a silent round — that is a
//!   configuration bug, and it must not appear as a price.
//! - **Level order is recorded, not assumed.** Each book entry carries the
//!   index it appeared at within its side, and `MDPriceLevel` (1023)
//!   overrides it when the venue sends one.

use crate::error::FixError;
use crate::frame::Frame;
use crate::tagvalue::MsgType;
use jeed_wire::{Scale, UnixNano};

/// Maximum market-data entries decoded from one message: a 10-deep
/// two-sided book plus room for trade / statistic entries.
pub const MD_MAX_ENTRIES: usize = 32;

/// Capacity of the inline `Symbol` / `MDReqID` buffers.
pub const FIX_TEXT_LEN: usize = 32;

/// Integer encoding applied to every price and size decoded from FIX text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixScales {
    /// Scale of `MDEntryPx` (270).
    pub price: Scale,

    /// Scale of `MDEntrySize` (271).
    pub quantity: Scale,
}

impl FixScales {
    /// Creates a scale pair.
    #[inline]
    pub const fn new(price: Scale, quantity: Scale) -> Self {
        Self { price, quantity }
    }
}

/// Short inline text field (`Symbol`, `MDReqID`). Values longer than
/// [`FIX_TEXT_LEN`] are truncated; that only affects identity strings the
/// engine matches on its own configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixText {
    bytes: [u8; FIX_TEXT_LEN],

    len: u8,
}

impl Default for FixText {
    #[inline]
    fn default() -> Self {
        Self::EMPTY
    }
}

impl FixText {
    /// Empty value.
    pub const EMPTY: Self = Self { bytes: [0; FIX_TEXT_LEN], len: 0 };

    /// Copies up to [`FIX_TEXT_LEN`] bytes.
    #[inline]
    pub fn new(value: &[u8]) -> Self {
        let mut out = Self::EMPTY;
        let n = value.len().min(FIX_TEXT_LEN);
        out.bytes[..n].copy_from_slice(&value[..n]);
        out.len = n as u8;
        out
    }

    /// The stored bytes.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    /// `true` when nothing was stored.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// `MDEntryType` (269).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MdEntryType {
    /// `0` — a bid level.
    Bid,

    /// `1` — an offer level.
    Offer,

    /// `2` — a trade print.
    Trade,

    /// `3` — index value.
    IndexValue,

    /// `4` — opening price.
    OpeningPrice,

    /// `5` — closing price.
    ClosingPrice,

    /// `6` — settlement price.
    SettlementPrice,

    /// `7` — session high.
    SessionHigh,

    /// `8` — session low.
    SessionLow,

    /// `9` — session VWAP.
    SessionVwap,

    /// `B` — traded volume.
    TradeVolume,

    /// `C` — open interest.
    OpenInterest,

    /// Any other single-byte value.
    Other {
        /// The raw byte.
        code: u8,
    },
}

impl MdEntryType {
    /// Decodes a `269` byte.
    #[inline]
    pub const fn from_byte(b: u8) -> Self {
        match b {
            b'0' => Self::Bid,
            b'1' => Self::Offer,
            b'2' => Self::Trade,
            b'3' => Self::IndexValue,
            b'4' => Self::OpeningPrice,
            b'5' => Self::ClosingPrice,
            b'6' => Self::SettlementPrice,
            b'7' => Self::SessionHigh,
            b'8' => Self::SessionLow,
            b'9' => Self::SessionVwap,
            b'B' => Self::TradeVolume,
            b'C' => Self::OpenInterest,
            code => Self::Other { code },
        }
    }

    /// The `269` byte. Inverse of [`from_byte`](Self::from_byte), including for
    /// [`Self::Other`], so a request can ask for an entry type this parser
    /// classifies as unknown.
    #[inline]
    pub const fn as_byte(self) -> u8 {
        match self {
            Self::Bid => b'0',
            Self::Offer => b'1',
            Self::Trade => b'2',
            Self::IndexValue => b'3',
            Self::OpeningPrice => b'4',
            Self::ClosingPrice => b'5',
            Self::SettlementPrice => b'6',
            Self::SessionHigh => b'7',
            Self::SessionLow => b'8',
            Self::SessionVwap => b'9',
            Self::TradeVolume => b'B',
            Self::OpenInterest => b'C',
            Self::Other { code } => code,
        }
    }

    /// `true` for [`Self::Bid`] and [`Self::Offer`].
    #[inline]
    pub const fn is_book_side(self) -> bool {
        matches!(self, Self::Bid | Self::Offer)
    }
}

/// `MDUpdateAction` (279). `35=W` entries carry no action; they are
/// [`Self::New`] by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MdUpdateAction {
    /// `0` — new entry.
    #[default]
    New,

    /// `1` — change an existing entry.
    Change,

    /// `2` — delete an entry.
    Delete,

    /// `3` — delete from the entry through the end of the book.
    DeleteThru,

    /// `4` — delete from the start of the book through the entry.
    DeleteFrom,

    /// `5` — overlay.
    Overlay,
}

impl MdUpdateAction {
    /// Decodes a `279` byte.
    #[inline]
    pub const fn from_byte(b: u8) -> Result<Self, FixError> {
        Ok(match b {
            b'0' => Self::New,
            b'1' => Self::Change,
            b'2' => Self::Delete,
            b'3' => Self::DeleteThru,
            b'4' => Self::DeleteFrom,
            b'5' => Self::Overlay,
            found => return Err(FixError::UnknownUpdateAction { found }),
        })
    }
}

/// One decoded `NoMDEntries` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MdEntry {
    /// `269` MDEntryType.
    pub entry_type: MdEntryType,

    /// `279` MDUpdateAction ([`MdUpdateAction::New`] on a snapshot).
    pub action: MdUpdateAction,

    /// `270` MDEntryPx on [`FixScales::price`]. Meaningful when
    /// [`Self::has_price`].
    pub price: i64,

    /// `271` MDEntrySize on [`FixScales::quantity`]. Meaningful when
    /// [`Self::has_size`].
    pub size: u64,

    /// `272` MDEntryDate as ns at UTC midnight (0 when absent).
    pub date_ns: UnixNano,

    /// `273` MDEntryTime as ns since midnight (0 when absent).
    pub time_ns: UnixNano,

    /// `346` NumberOfOrders. Meaningful when [`Self::has_order_count`].
    pub order_count: u32,

    /// Zero-based book level: `1023` MDPriceLevel minus one when the venue
    /// sends it, otherwise the entry's position within its own side.
    pub level: u8,

    /// `55` Symbol of this entry, empty when the message carries it once in
    /// the header (the usual case for `35=W`).
    pub symbol: FixText,

    /// `270` was present.
    pub has_price: bool,

    /// `271` was present.
    pub has_size: bool,

    /// `272` was present.
    pub has_date: bool,

    /// `273` was present.
    pub has_time: bool,

    /// `346` was present.
    pub has_order_count: bool,

    /// `1023` was present.
    pub has_explicit_level: bool,
}

impl Default for MdEntry {
    #[inline]
    fn default() -> Self {
        Self::EMPTY
    }
}

impl MdEntry {
    /// Entry with no field set. `entry_type` is a placeholder until `269`
    /// is read.
    pub const EMPTY: Self = Self {
        entry_type: MdEntryType::Other { code: 0 },
        action: MdUpdateAction::New,
        price: 0,
        size: 0,
        date_ns: 0,
        time_ns: 0,
        order_count: 0,
        level: 0,
        symbol: FixText::EMPTY,
        has_price: false,
        has_size: false,
        has_date: false,
        has_time: false,
        has_order_count: false,
        has_explicit_level: false,
    };

    /// Absolute exchange time from `272` + `273`, when both were sent.
    ///
    /// This is the venue clock. `SendingTime` (52) is the *counterparty's*
    /// clock and must not be used in its place.
    #[inline]
    pub const fn venue_time(&self) -> Option<UnixNano> {
        if self.has_date && self.has_time {
            Some(self.date_ns + self.time_ns)
        } else {
            None
        }
    }
}

/// A decoded market-data message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MdMessage {
    /// `35` MsgType ([`MsgType::MarketDataSnapshot`] or
    /// [`MsgType::MarketDataIncremental`]).
    pub msg_type: MsgType,

    /// `34` MsgSeqNum.
    pub msg_seq_num: u64,

    /// `52` SendingTime — the counterparty's clock, for latency measurement
    /// only.
    pub sending_time: UnixNano,

    /// `262` MDReqID.
    pub req_id: FixText,

    /// `55` Symbol from the message header (empty when only entries carry it).
    pub symbol: FixText,

    /// `268` NoMDEntries as declared.
    pub declared_entries: u32,

    /// `43` PossDupFlag.
    pub poss_dup: bool,

    entries: [MdEntry; MD_MAX_ENTRIES],

    count: u8,
}

impl Default for MdMessage {
    #[inline]
    fn default() -> Self {
        Self::EMPTY
    }
}

impl MdMessage {
    /// Message with no entries.
    pub const EMPTY: Self = Self {
        msg_type: MsgType::Other { first: 0 },
        msg_seq_num: 0,
        sending_time: 0,
        req_id: FixText::EMPTY,
        symbol: FixText::EMPTY,
        declared_entries: 0,
        poss_dup: false,
        entries: [MdEntry::EMPTY; MD_MAX_ENTRIES],
        count: 0,
    };

    /// The decoded entries in wire order.
    #[inline]
    pub fn entries(&self) -> &[MdEntry] {
        &self.entries[..self.count as usize]
    }

    /// Number of decoded entries.
    #[inline]
    pub const fn len(&self) -> usize {
        self.count as usize
    }

    /// `true` when the message carried no entries.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// `true` for `35=W`.
    #[inline]
    pub const fn is_snapshot(&self) -> bool {
        matches!(self.msg_type, MsgType::MarketDataSnapshot)
    }

    /// Symbol of `entry`: its own `55` when present, else the message's.
    #[inline]
    pub fn entry_symbol<'a>(&'a self, entry: &'a MdEntry) -> &'a [u8] {
        if entry.symbol.is_empty() {
            self.symbol.as_bytes()
        } else {
            entry.symbol.as_bytes()
        }
    }

    /// Venue time of the message: the first entry that carries `272`+`273`.
    #[inline]
    pub fn venue_time(&self) -> Option<UnixNano> {
        self.entries().iter().find_map(MdEntry::venue_time)
    }

    /// Highest `level + 1` over the book entries of one side — the depth the
    /// message actually describes.
    pub fn side_depth(&self, side: MdEntryType) -> usize {
        self.entries()
            .iter()
            .filter(|e| e.entry_type == side)
            .map(|e| e.level as usize + 1)
            .max()
            .unwrap_or(0)
    }
}

// ============================================================================
// Tags
// ============================================================================

/// The market-data and header tags this module reads.
pub mod tags {
    /// MsgType.
    pub const MSG_TYPE: u32 = 35;
    /// MsgSeqNum.
    pub const MSG_SEQ_NUM: u32 = 34;
    /// SendingTime.
    pub const SENDING_TIME: u32 = 52;
    /// PossDupFlag.
    pub const POSS_DUP_FLAG: u32 = 43;
    /// Symbol.
    pub const SYMBOL: u32 = 55;
    /// MDReqID.
    pub const MD_REQ_ID: u32 = 262;
    /// NoMDEntries.
    pub const NO_MD_ENTRIES: u32 = 268;
    /// MDEntryType.
    pub const MD_ENTRY_TYPE: u32 = 269;
    /// MDEntryPx.
    pub const MD_ENTRY_PX: u32 = 270;
    /// MDEntrySize.
    pub const MD_ENTRY_SIZE: u32 = 271;
    /// MDEntryDate.
    pub const MD_ENTRY_DATE: u32 = 272;
    /// MDEntryTime.
    pub const MD_ENTRY_TIME: u32 = 273;
    /// MDUpdateAction.
    pub const MD_UPDATE_ACTION: u32 = 279;
    /// NumberOfOrders.
    pub const NUMBER_OF_ORDERS: u32 = 346;
    /// MDPriceLevel.
    pub const MD_PRICE_LEVEL: u32 = 1023;
}

/// `true` when `tag` belongs to the `NoMDEntries` group.
#[inline]
const fn is_group_tag(tag: u32) -> bool {
    matches!(
        tag,
        tags::MD_ENTRY_TYPE
            | tags::MD_ENTRY_PX
            | tags::MD_ENTRY_SIZE
            | tags::MD_ENTRY_DATE
            | tags::MD_ENTRY_TIME
            | tags::MD_UPDATE_ACTION
            | tags::NUMBER_OF_ORDERS
            | tags::MD_PRICE_LEVEL
    )
}

/// Decodes a framed `35=W` / `35=X` message.
///
/// The group delimiter is `279` for an incremental refresh and `269` for a
/// snapshot; a repeated field inside one entry also opens the next, so a
/// venue that orders the group differently still decodes. `Symbol` (55) is
/// taken as the message's before the first entry and as the entry's inside
/// the group, which is how `35=X` carries several instruments.
pub fn parse_md_message(frame: &Frame<'_>, scales: FixScales) -> Result<MdMessage, FixError> {
    let mut msg = MdMessage::EMPTY;
    let mut cur: Option<MdEntry> = None;
    let mut bid_seen = 0u8;
    let mut ask_seen = 0u8;
    let mut have_msg_type = false;
    let mut have_seq = false;

    for field in frame.fields() {
        let field = field?;
        let tag = field.tag;

        if is_group_tag(tag) {
            let delimiter = match msg.msg_type {
                MsgType::MarketDataIncremental => tags::MD_UPDATE_ACTION,
                _ => tags::MD_ENTRY_TYPE,
            };
            let repeat = match &cur {
                Some(e) => match tag {
                    tags::MD_ENTRY_TYPE => e.entry_type != MdEntryType::Other { code: 0 },
                    tags::MD_ENTRY_PX => e.has_price,
                    tags::MD_ENTRY_SIZE => e.has_size,
                    tags::MD_ENTRY_DATE => e.has_date,
                    tags::MD_ENTRY_TIME => e.has_time,
                    tags::NUMBER_OF_ORDERS => e.has_order_count,
                    tags::MD_PRICE_LEVEL => e.has_explicit_level,
                    _ => false,
                },
                None => false,
            };
            if tag == delimiter || repeat {
                if let Some(done) = cur.take() {
                    push_entry(&mut msg, done, &mut bid_seen, &mut ask_seen)?;
                }
                cur = Some(MdEntry::EMPTY);
            }
            let entry = cur.as_mut().ok_or(FixError::GroupTagOutsideEntry { tag })?;
            match tag {
                tags::MD_ENTRY_TYPE => entry.entry_type = MdEntryType::from_byte(field.first_byte()?),
                tags::MD_UPDATE_ACTION => {
                    entry.action = MdUpdateAction::from_byte(field.first_byte()?)?;
                }
                tags::MD_ENTRY_PX => {
                    entry.price = field.as_scaled(scales.price)?;
                    entry.has_price = true;
                }
                tags::MD_ENTRY_SIZE => {
                    entry.size = field.as_scaled_unsigned(scales.quantity)?;
                    entry.has_size = true;
                }
                tags::MD_ENTRY_DATE => {
                    entry.date_ns = field.as_utc_date()?;
                    entry.has_date = true;
                }
                tags::MD_ENTRY_TIME => {
                    entry.time_ns = field.as_utc_time()?;
                    entry.has_time = true;
                }
                tags::NUMBER_OF_ORDERS => {
                    entry.order_count = field.as_u32()?;
                    entry.has_order_count = true;
                }
                tags::MD_PRICE_LEVEL => {
                    let level = field.as_u32()?;
                    entry.level = level.saturating_sub(1).min(u8::MAX as u32) as u8;
                    entry.has_explicit_level = true;
                }
                _ => {}
            }
            continue;
        }

        match tag {
            tags::MSG_TYPE => {
                msg.msg_type = MsgType::from_value(field.value);
                if !msg.msg_type.is_market_data() {
                    return Err(FixError::UnexpectedMsgType {
                        found: field.value.first().copied().unwrap_or(0),
                    });
                }
                have_msg_type = true;
            }
            tags::MSG_SEQ_NUM => {
                msg.msg_seq_num = field.as_u64()?;
                have_seq = true;
            }
            tags::SENDING_TIME => msg.sending_time = field.as_utc_timestamp()?,
            tags::POSS_DUP_FLAG => msg.poss_dup = field.is_yes(),
            tags::MD_REQ_ID => msg.req_id = FixText::new(field.value),
            tags::NO_MD_ENTRIES => msg.declared_entries = field.as_u32()?,
            tags::SYMBOL => match cur.as_mut() {
                Some(entry) => entry.symbol = FixText::new(field.value),
                None => msg.symbol = FixText::new(field.value),
            },
            _ => {}
        }
    }

    if let Some(done) = cur.take() {
        push_entry(&mut msg, done, &mut bid_seen, &mut ask_seen)?;
    }
    if !have_msg_type {
        return Err(FixError::MissingTag { tag: tags::MSG_TYPE });
    }
    if !have_seq {
        return Err(FixError::MissingTag { tag: tags::MSG_SEQ_NUM });
    }
    if msg.declared_entries != msg.count as u32 {
        return Err(FixError::EntryCount {
            declared: msg.declared_entries,
            found: msg.count as u32,
        });
    }
    Ok(msg)
}

/// Appends a finished entry, assigning its implied level.
fn push_entry(
    msg: &mut MdMessage,
    mut entry: MdEntry,
    bid_seen: &mut u8,
    ask_seen: &mut u8,
) -> Result<(), FixError> {
    if entry.entry_type == (MdEntryType::Other { code: 0 }) {
        return Err(FixError::MissingTag { tag: tags::MD_ENTRY_TYPE });
    }
    if !entry.has_explicit_level {
        entry.level = match entry.entry_type {
            MdEntryType::Bid => std::mem::replace(bid_seen, bid_seen.saturating_add(1)),
            MdEntryType::Offer => std::mem::replace(ask_seen, ask_seen.saturating_add(1)),
            _ => 0,
        };
    } else {
        match entry.entry_type {
            MdEntryType::Bid => *bid_seen = bid_seen.saturating_add(1),
            MdEntryType::Offer => *ask_seen = ask_seen.saturating_add(1),
            _ => {}
        }
    }
    let idx = msg.count as usize;
    if idx >= MD_MAX_ENTRIES {
        return Err(FixError::TooManyEntries {
            declared: msg.declared_entries.max(idx as u32 + 1),
            max: MD_MAX_ENTRIES as u32,
        });
    }
    msg.entries[idx] = entry;
    msg.count += 1;
    Ok(())
}
