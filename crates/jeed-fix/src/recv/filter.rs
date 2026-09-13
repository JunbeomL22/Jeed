//! The symbol allow-set the receive loop applies before the adapter runs.
//!
//! ## One filter, where KRX has three
//!
//! `jeed_krx::recv::filter` stacks IGMP join → trcode → ISIN because multicast
//! pushes everything at you: an options port carries every strike whether the
//! handler asked or not (`documents/todo.md` §5).
//!
//! A FIX session carries what we **subscribed** to. The `35=V` we sent is the
//! real filter, it runs at the venue, and it costs us nothing. So this set is a
//! backstop for the venue that ignores a subscription or sends a whole board
//! down one session — and that is why it defaults to keeping everything, where
//! KRX's trcode set defaults to keeping nothing. An empty `symbols` in conf
//! there is a typo that should produce a loud silent feed; here it is the
//! ordinary case.
//!
//! ## A message passes if *any* of its symbols does
//!
//! `35=W` names its instrument once in the header; `35=X` can carry several
//! inside the group, one per entry. A message-level verdict is therefore the
//! only one this stage can give, and per-entry selection belongs to the
//! [`MdAdapter`](super::MdAdapter) — which is the code that knows how entries
//! become records, and can reach the same set through
//! [`Pipeline::symbols`](super::Pipeline::symbols).

use crate::market_data::{FIX_TEXT_LEN, MdMessage};

/// The `Symbol` (55) values to keep. **Empty keeps everything.**
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymbolFilter {
    /// Sorted, so a lookup is a binary search. Each entry is the symbol bytes
    /// padded with zeros, which sorts identically to the symbols themselves
    /// because no symbol contains a zero byte.
    allow: Box<[[u8; FIX_TEXT_LEN]]>,
}

impl SymbolFilter {
    /// Keeps every instrument on the session.
    #[inline]
    pub fn all() -> Self {
        Self::default()
    }

    /// Keeps exactly these symbols. Duplicates are collapsed; a symbol longer
    /// than [`FIX_TEXT_LEN`] is truncated to the same prefix the decoder keeps,
    /// so the two agree on what they are comparing.
    pub fn new<S: AsRef<[u8]>>(symbols: impl IntoIterator<Item = S>) -> Self {
        let mut allow: Vec<[u8; FIX_TEXT_LEN]> = symbols
            .into_iter()
            .map(|s| {
                let s = s.as_ref();
                let mut out = [0u8; FIX_TEXT_LEN];
                let n = s.len().min(FIX_TEXT_LEN);
                out[..n].copy_from_slice(&s[..n]);
                out
            })
            .collect();
        allow.sort_unstable();
        allow.dedup();
        Self { allow: allow.into_boxed_slice() }
    }

    /// `true` if this symbol is kept.
    #[inline]
    pub fn allows(&self, symbol: &[u8]) -> bool {
        if self.allow.is_empty() {
            return true;
        }
        if symbol.len() > FIX_TEXT_LEN {
            return false;
        }
        let mut key = [0u8; FIX_TEXT_LEN];
        key[..symbol.len()].copy_from_slice(symbol);
        self.allow.binary_search(&key).is_ok()
    }

    /// `true` if any symbol the message names is kept.
    ///
    /// A message with no `55` anywhere passes: the venue is identifying the
    /// instrument by `MDReqID` alone, and refusing it here would be this stage
    /// deciding something only the adapter can know.
    pub fn allows_message(&self, msg: &MdMessage) -> bool {
        if self.allow.is_empty() {
            return true;
        }
        if !msg.symbol.is_empty() && self.allows(msg.symbol.as_bytes()) {
            return true;
        }
        let mut named = !msg.symbol.is_empty();
        for entry in msg.entries() {
            if entry.symbol.is_empty() {
                continue;
            }
            named = true;
            if self.allows(entry.symbol.as_bytes()) {
                return true;
            }
        }
        !named
    }

    /// Number of symbols listed. Zero means "keep everything".
    #[inline]
    pub fn len(&self) -> usize {
        self.allow.len()
    }

    /// `true` when nothing is listed, i.e. everything is kept.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.allow.is_empty()
    }

    /// The symbols listed, sorted, without their zero padding.
    pub fn symbols(&self) -> impl Iterator<Item = &[u8]> {
        self.allow.iter().map(|s| {
            let end = s.iter().position(|&b| b == 0).unwrap_or(FIX_TEXT_LEN);
            &s[..end]
        })
    }
}

/// Reads an allow-list: one symbol per line, `#` comments and blank lines
/// ignored.
///
/// Unlike `jeed_krx::recv::parse_isin_list` this cannot fail. A KRX 종목코드 is
/// twelve bytes and a line that is not is a corrupt file; a FIX symbol has no
/// fixed width and nothing here can tell a venue's spelling from a typo. A
/// symbol that no message ever carries shows up as a quiet feed, which is the
/// same way a mis-assigned KRX port shows up.
pub fn parse_symbol_list(text: &str) -> SymbolFilter {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = match line.split_once('#') {
            Some((head, _)) => head,
            None => line,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        out.push(line.as_bytes().to_vec());
    }
    SymbolFilter::new(out)
}
