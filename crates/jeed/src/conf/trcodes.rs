//! `conf/krx_trcodes.toml` — the generated table of every trcode the two
//! standards name.
//!
//! Only the parts start-up validation reads are kept: which codes exist and
//! which interface each maps to. The lengths in `[interface]` are **not**
//! compared against the decoders' — the decoders' lengths come from
//! `documents/krx/layouts.md` and the pcap, and for `04F` the standard is
//! known to be wrong (`CLAUDE.md`, "주식선물은 10단 상품인데 시세는 5단으로
//! 온다"). A check that fires on a known discrepancy every start-up would be
//! a check that gets ignored.
//!
//! ## Absent is a warning
//!
//! A code in `conf/krx.toml` that this table does not know is *probably* a
//! typo and *possibly* a product the standard has not caught up with —
//! `17F` arrived on the circuit before it appeared in v1.341
//! (`documents/todo.md` §14②). So the handler warns and runs.

use crate::conf::{ConfError, Section, read};
use crate::toml::Table;
use jeed_krx::TrCode;
use std::collections::BTreeMap;
use std::path::Path;

/// The trcode table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrCodeTable {
    /// `spec_version` — the 정보분배 standard the table was generated from.
    pub spec_version: String,

    /// `channel_spec_version` — the 송신채널 standard.
    pub channel_spec_version: String,

    /// trcode → interface id, from `[code]`.
    codes: BTreeMap<u64, String>,

    /// trcodes from `[code_by_channel]` — known, but their interface depends
    /// on which circuit they arrive on.
    by_channel: Vec<u64>,
}

impl TrCodeTable {
    /// Reads the table from a file.
    pub fn load(path: &Path) -> Result<Self, ConfError> {
        Self::from_table(&read(path)?)
    }

    /// Reads the table from parsed TOML.
    pub fn from_table(table: &Table) -> Result<Self, ConfError> {
        let root = Section::new("", table);
        let spec_version = root.optional_str("spec_version")?.unwrap_or("").to_owned();
        let channel_spec_version = root.optional_str("channel_spec_version")?.unwrap_or("").to_owned();

        let mut codes = BTreeMap::new();
        if let Some(section) = root.optional_table("code")? {
            for (key, value) in section.table().iter() {
                let code = trcode(&section, key)?;
                let entry = value.as_table().map(|t| Section::new(key, t));
                let interface = entry
                    .and_then(|e| e.optional_str("interface").ok().flatten())
                    .unwrap_or("")
                    .to_owned();
                codes.insert(code.as_u64(), interface);
            }
        }

        let mut by_channel = Vec::new();
        if let Some(section) = root.optional_table("code_by_channel")? {
            for key in section.table().keys() {
                by_channel.push(trcode(&section, key)?.as_u64());
            }
        }

        Ok(Self { spec_version, channel_spec_version, codes, by_channel })
    }

    /// `true` if either standard names `code`.
    pub fn knows(&self, code: TrCode) -> bool {
        let k = code.as_u64();
        self.codes.contains_key(&k) || self.by_channel.contains(&k)
    }

    /// The interface id for `code`, when it is one fixed interface.
    pub fn interface_of(&self, code: TrCode) -> Option<&str> {
        self.codes.get(&code.as_u64()).map(String::as_str)
    }

    /// Codes in `[code]`.
    #[inline]
    pub fn len(&self) -> usize {
        self.codes.len()
    }

    /// `true` if `[code]` is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.codes.is_empty()
    }
}

/// A five-byte key, or an error naming the section.
pub(crate) fn trcode(section: &Section<'_>, key: &str) -> Result<TrCode, ConfError> {
    let bytes: [u8; jeed_krx::TRCODE_LEN] = key
        .as_bytes()
        .try_into()
        .map_err(|_| section.invalid(key, "is not a five-byte trcode"))?;
    Ok(TrCode::new(bytes))
}
