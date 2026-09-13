//! The two allow-sets the receive loop applies before a ring slot is claimed.
//!
//! | 층 | 단위 | 비용 | 왜 |
//! |---|---|---|---|
//! | IGMP 가입 | 상품군 / 분배그룹 | 0 — the NIC never sees it | [`Endpoint`](super::Endpoint) |
//! | trcode | 데이터구분 × 상품군 | one binary search over `u64` | [`TrCodeFilter`] |
//! | ISIN | 종목 | one binary search over 12 bytes | [`IsinFilter`] |
//!
//! **These are real filters, not configuration checks.** One port carries every
//! data class of a product group and every instrument in it — an options port
//! carries every strike — so a handler that wants `B601F` receives `A7`, `O6`,
//! `R1` and the rest whether it asked for them or not (`documents/todo.md` §5).
//!
//! ## The two are not symmetric, on purpose
//!
//! An **empty trcode set keeps nothing**: `trcodes` in `conf` is the list of
//! things to salvage from a stream of mostly-unwanted messages, and a typo that
//! emptied it should produce a silent feed, which is loud, rather than a ring
//! flooded with every data class on the wire, which looks like it is working.
//!
//! An **empty ISIN set keeps everything**: most deployments want the whole
//! product group, and writing out every strike would be a conf file nobody
//! maintains.

use crate::trcode::TrCode;
use jeed_wire::{ISIN_LEN, Isin};

/// The 데이터구분 codes to keep.
///
/// Stored as the `u64` packing of the five bytes ([`TrCode::as_u64`]), sorted,
/// so a lookup is a binary search over integers rather than slice comparisons.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrCodeFilter {
    keys: Box<[u64]>,
    codes: Box<[TrCode]>,
}

impl TrCodeFilter {
    /// Keeps nothing. Every datagram is dropped by the trcode layer.
    #[inline]
    pub fn none() -> Self {
        Self::default()
    }

    /// Keeps exactly these codes. Duplicates are collapsed.
    pub fn new(codes: impl IntoIterator<Item = TrCode>) -> Self {
        let mut codes: Vec<TrCode> = codes.into_iter().collect();
        codes.sort_unstable_by_key(|c| c.as_u64());
        codes.dedup();
        let keys = codes.iter().map(|c| c.as_u64()).collect::<Vec<_>>();
        Self { keys: keys.into_boxed_slice(), codes: codes.into_boxed_slice() }
    }

    /// Position of `code` in the set, or `None` if it is not kept.
    ///
    /// The *position* is returned rather than a `bool` because the per-socket
    /// wiring counters key off it: "this trcode was configured but has never
    /// arrived on this socket" is how a mis-assigned port becomes visible, and
    /// the socket ↔ trcode mapping is the one thing startup cannot validate
    /// (`documents/todo.md` §5).
    #[inline]
    pub fn index_of(&self, code: TrCode) -> Option<usize> {
        self.keys.binary_search(&code.as_u64()).ok()
    }

    /// `true` if `code` is kept.
    #[inline]
    pub fn keeps(&self, code: TrCode) -> bool {
        self.index_of(code).is_some()
    }

    /// The code at `index`, as returned by [`index_of`](Self::index_of).
    #[inline]
    pub fn code(&self, index: usize) -> Option<TrCode> {
        self.codes.get(index).copied()
    }

    /// Every code in the set, in lookup order.
    #[inline]
    pub fn codes(&self) -> &[TrCode] {
        &self.codes
    }

    /// Number of codes kept.
    #[inline]
    pub fn len(&self) -> usize {
        self.codes.len()
    }

    /// `true` when the set keeps nothing.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.codes.is_empty()
    }
}

impl FromIterator<TrCode> for TrCodeFilter {
    fn from_iter<I: IntoIterator<Item = TrCode>>(iter: I) -> Self {
        Self::new(iter)
    }
}

/// The 종목코드 values to keep. **Empty keeps everything.**
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IsinFilter {
    allow: Box<[Isin]>,
}

impl IsinFilter {
    /// Keeps every instrument.
    #[inline]
    pub fn all() -> Self {
        Self::default()
    }

    /// Keeps exactly these instruments. Duplicates are collapsed.
    pub fn new(isins: impl IntoIterator<Item = Isin>) -> Self {
        let mut allow: Vec<Isin> = isins.into_iter().collect();
        allow.sort_unstable();
        allow.dedup();
        Self { allow: allow.into_boxed_slice() }
    }

    /// `true` if this instrument is kept.
    ///
    /// `isin` is the raw field read straight out of the datagram, so a slice of
    /// the wrong length is a caller bug and is refused rather than padded.
    #[inline]
    pub fn allows(&self, isin: &[u8]) -> bool {
        if self.allow.is_empty() {
            return true;
        }
        let Ok(isin) = <&Isin>::try_from(isin) else {
            return false;
        };
        self.allow.binary_search(isin).is_ok()
    }

    /// Number of instruments listed. Zero means "keep everything".
    #[inline]
    pub fn len(&self) -> usize {
        self.allow.len()
    }

    /// `true` when nothing is listed, i.e. everything is kept.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.allow.is_empty()
    }

    /// The instruments listed, sorted.
    #[inline]
    pub fn isins(&self) -> &[Isin] {
        &self.allow
    }
}

impl FromIterator<Isin> for IsinFilter {
    fn from_iter<I: IntoIterator<Item = Isin>>(iter: I) -> Self {
        Self::new(iter)
    }
}

/// Reads an allow-list file: one 종목코드 per line, `#` comments and blank
/// lines ignored.
///
/// The file is a deployment artefact rather than part of `conf` because it is
/// regenerated per trading day for option chains.
pub fn parse_isin_list(text: &str) -> Result<IsinFilter, IsinListError> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = match line.split_once('#') {
            Some((head, _)) => head,
            None => line,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let isin = <&Isin>::try_from(line.as_bytes())
            .map_err(|_| IsinListError { line: n + 1, len: line.len() })?;
        out.push(*isin);
    }
    Ok(IsinFilter::new(out))
}

/// A line of an ISIN allow-list was not a 12-byte 종목코드.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IsinListError {
    /// One-based line number.
    pub line: usize,

    /// Length found, where [`ISIN_LEN`] was required.
    pub len: usize,
}

impl core::fmt::Display for IsinListError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "line {}: {} bytes, expected {ISIN_LEN}", self.line, self.len)
    }
}

impl std::error::Error for IsinListError {}
