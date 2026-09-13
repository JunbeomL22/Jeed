//! Deployment configuration — the typed view over [`toml`].
//!
//! ```text
//! conf/krx.toml ────→ toml::parse ──→ Table ──→ KrxConf ────→ validate ──→ start
//!                                                   ↑
//! conf/krx_trcodes.toml ──→ TrCodeTable ────────────┘  (warnings only)
//! conf/crypto.toml ─→ toml::parse ──→ Table ──→ CryptoConf ─→ validate ──→ start
//! ```
//!
//! The placement rules — cores, rings, SMT siblings — are one module,
//! [`rules`], because a KRX feed and a crypto feed are placed the same way.
//!
//! ## Strict on keys, explicit on zero
//!
//! An unknown key is an error, not a warning: `ring_slot = 16384` next to a
//! default `ring_slots` would otherwise run with the wrong size and say
//! nothing. And a guard that is absent is on, at a conservative value —
//! `stale_ms` left out once left a book frozen for twenty minutes
//! (`documents/feed_handler.md` §9). Turning one off means writing `0`.
//!
//! ## Paths are relative to the working directory
//!
//! `trcode_table = "conf/krx_trcodes.toml"` is read as written, from wherever
//! the binary was started. The example conf assumes the repository root. A
//! service wrapper that starts the handler elsewhere sets the directory, not
//! the conf.

pub mod crypto;
pub mod fix;
pub mod krx;
pub mod rules;
pub mod trcodes;

use crate::toml::{self, Table, Value};
use core::fmt;
use std::path::{Path, PathBuf};

pub use crypto::{CryptoConf, CryptoFeedConf, InstrumentConf};
pub use fix::{FixConf, FixFeedConf};
pub use krx::{FeedConf, KrxConf, Warning};
pub use rules::RuleError;
pub use trcodes::TrCodeTable;

/// The `[health]` section, shared by every feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Health {
    /// Liveness record cadence. Zero disables it.
    pub heartbeat_ns: u64,

    /// Age past which a record is flagged `STALE`. Zero disables it.
    pub stale_ns: u64,
}

impl Default for Health {
    /// 100 ms heartbeat, 500 ms stale — §9's recommended initial values, and
    /// what a conf without a `[health]` section gets.
    fn default() -> Self {
        Self { heartbeat_ns: 100_000_000, stale_ns: 500_000_000 }
    }
}

impl Health {
    /// Reads `[health]`, or the defaults if the section is absent.
    pub fn from_section(section: Option<Section<'_>>) -> Result<Self, ConfError> {
        let Some(s) = section else {
            return Ok(Self::default());
        };
        let d = Self::default();
        let health = Self {
            heartbeat_ns: s.optional_u64("heartbeat_ms")?.map_or(d.heartbeat_ns, |ms| ms * 1_000_000),
            stale_ns: s.optional_u64("stale_ms")?.map_or(d.stale_ns, |ms| ms * 1_000_000),
        };
        s.finish(&["heartbeat_ms", "stale_ms"])?;
        Ok(health)
    }
}

/// Reads and parses a TOML file.
pub fn read(path: &Path) -> Result<Table, ConfError> {
    let text = std::fs::read_to_string(path)
        .map_err(|source| ConfError::Io { path: path.to_owned(), source })?;
    toml::parse(&text).map_err(|source| ConfError::Toml { path: path.to_owned(), source })
}

/// A table with a name, for error messages that say where.
#[derive(Debug, Clone, Copy)]
pub struct Section<'a> {
    /// `feed[hot]`, `health`, or `` for the root.
    pub at: &'a str,
    table: &'a Table,
}

impl<'a> Section<'a> {
    /// Wraps a table under a name.
    pub const fn new(at: &'a str, table: &'a Table) -> Self {
        Self { at, table }
    }

    /// The underlying table.
    pub const fn table(&self) -> &'a Table {
        self.table
    }

    fn missing(&self, key: &'static str) -> ConfError {
        ConfError::Missing { at: self.at.to_owned(), key }
    }

    fn wrong_type(&self, key: &str, expected: &'static str, found: &Value) -> ConfError {
        ConfError::Type {
            at: self.at.to_owned(),
            key: key.to_owned(),
            expected,
            found: found.type_name(),
        }
    }

    /// `key = "…"`, required.
    pub fn required_str(&self, key: &'static str) -> Result<&'a str, ConfError> {
        self.optional_str(key)?.ok_or_else(|| self.missing(key))
    }

    /// `key = "…"`, optional.
    pub fn optional_str(&self, key: &str) -> Result<Option<&'a str>, ConfError> {
        match self.table.get(key) {
            None => Ok(None),
            Some(v) => v.as_str().map(Some).ok_or_else(|| self.wrong_type(key, "a string", v)),
        }
    }

    /// `key = 123`, required, non-negative.
    pub fn required_u64(&self, key: &'static str) -> Result<u64, ConfError> {
        self.optional_u64(key)?.ok_or_else(|| self.missing(key))
    }

    /// `key = 123`, optional, non-negative.
    pub fn optional_u64(&self, key: &str) -> Result<Option<u64>, ConfError> {
        match self.table.get(key) {
            None => Ok(None),
            Some(v) => match v.as_integer() {
                Some(n) if n >= 0 => Ok(Some(n as u64)),
                Some(_) => Err(self.invalid(key, "must not be negative")),
                None => Err(self.wrong_type(key, "an integer", v)),
            },
        }
    }

    /// `key = true`, optional.
    pub fn optional_bool(&self, key: &str) -> Result<Option<bool>, ConfError> {
        match self.table.get(key) {
            None => Ok(None),
            Some(v) => v.as_bool().map(Some).ok_or_else(|| self.wrong_type(key, "a boolean", v)),
        }
    }

    /// `key = ["…", "…"]`, required (an empty array is fine here; whether it
    /// may be empty is the caller's rule).
    pub fn required_strs(&self, key: &'static str) -> Result<Vec<&'a str>, ConfError> {
        let v = self.table.get(key).ok_or_else(|| self.missing(key))?;
        let items = v.as_array().ok_or_else(|| self.wrong_type(key, "an array of strings", v))?;
        items
            .iter()
            .map(|item| item.as_str().ok_or_else(|| self.wrong_type(key, "an array of strings", item)))
            .collect()
    }

    /// `key = [1, 2, 3]`, required, each in `u16`.
    pub fn required_u16s(&self, key: &'static str) -> Result<Vec<u16>, ConfError> {
        let v = self.table.get(key).ok_or_else(|| self.missing(key))?;
        let items = v.as_array().ok_or_else(|| self.wrong_type(key, "an array of integers", v))?;
        items
            .iter()
            .map(|item| match item.as_integer() {
                Some(n) if (0..=i64::from(u16::MAX)).contains(&n) => Ok(n as u16),
                Some(_) => Err(self.invalid(key, "out of range")),
                None => Err(self.wrong_type(key, "an array of integers", item)),
            })
            .collect()
    }

    /// `[key]`, optional.
    pub fn optional_table(&self, key: &'a str) -> Result<Option<Section<'a>>, ConfError> {
        match self.table.get(key) {
            None => Ok(None),
            Some(v) => v
                .as_table()
                .map(|t| Some(Section::new(key, t)))
                .ok_or_else(|| self.wrong_type(key, "a table", v)),
        }
    }

    /// `[[key]]`, zero or more.
    pub fn tables(&self, key: &str) -> Result<Vec<&'a Table>, ConfError> {
        match self.table.get(key) {
            None => Ok(Vec::new()),
            Some(v) => {
                let items = v.as_array().ok_or_else(|| self.wrong_type(key, "an array of tables", v))?;
                items
                    .iter()
                    .map(|item| item.as_table().ok_or_else(|| self.wrong_type(key, "an array of tables", item)))
                    .collect()
            }
        }
    }

    /// An error about `key` that is not about its type.
    pub fn invalid(&self, key: &str, reason: impl Into<String>) -> ConfError {
        ConfError::Invalid { at: self.at.to_owned(), key: key.to_owned(), reason: reason.into() }
    }

    /// Fails on any key not in `known` — the typo check.
    pub fn finish(&self, known: &[&str]) -> Result<(), ConfError> {
        for key in self.table.keys() {
            if !known.contains(&key) {
                return Err(ConfError::Unknown { at: self.at.to_owned(), key: key.to_owned() });
            }
        }
        Ok(())
    }
}

/// Why a conf was refused.
#[derive(Debug)]
pub enum ConfError {
    /// The file could not be read.
    Io {
        /// Which file.
        path: PathBuf,
        /// What the OS said.
        source: std::io::Error,
    },

    /// The file is not TOML this reader accepts.
    Toml {
        /// Which file.
        path: PathBuf,
        /// Where and why.
        source: toml::Error,
    },

    /// A required key is absent.
    Missing {
        /// Section.
        at: String,
        /// Key.
        key: &'static str,
    },

    /// A key holds the wrong kind of value.
    Type {
        /// Section.
        at: String,
        /// Key.
        key: String,
        /// What was wanted.
        expected: &'static str,
        /// What was there.
        found: &'static str,
    },

    /// A key holds a value of the right kind that still cannot be used.
    Invalid {
        /// Section.
        at: String,
        /// Key.
        key: String,
        /// Why not.
        reason: String,
    },

    /// A key this build does not know — most likely a typo.
    Unknown {
        /// Section.
        at: String,
        /// Key.
        key: String,
    },

    /// A rule that spans keys or sections was broken.
    Rule(RuleError),
}

impl fmt::Display for ConfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Toml { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Missing { at, key } => write!(f, "{}: `{key}` is required", where_(at)),
            Self::Type { at, key, expected, found } => {
                write!(f, "{}: `{key}` must be {expected}, found {found}", where_(at))
            }
            Self::Invalid { at, key, reason } => write!(f, "{}: `{key}` {reason}", where_(at)),
            Self::Unknown { at, key } => write!(f, "{}: unknown key `{key}`", where_(at)),
            Self::Rule(r) => write!(f, "{r}"),
        }
    }
}

fn where_(at: &str) -> &str {
    if at.is_empty() { "top level" } else { at }
}

impl std::error::Error for ConfError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Toml { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<RuleError> for ConfError {
    fn from(r: RuleError) -> Self {
        Self::Rule(r)
    }
}
