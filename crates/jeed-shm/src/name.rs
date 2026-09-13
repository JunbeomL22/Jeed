//! Validated Win32 section names.
//!
//! A section name is a kernel object name, not a path. It is built once at
//! startup and held as a NUL-terminated UTF-16 buffer so that opening or
//! re-opening a segment never allocates.

use crate::error::ShmError;
use core::fmt;

/// Longest section name accepted, excluding the namespace prefix.
pub const MAX_NAME_LEN: usize = 200;

/// Session-local namespace. Needs no privilege.
const LOCAL: &str = "Local\\";

/// Cross-session namespace. Creating here needs `SeCreateGlobalPrivilege`,
/// which an interactive process does not have by default.
const GLOBAL: &str = "Global\\";

/// Prefix + name + NUL.
const WIDE_CAP: usize = 7 + MAX_NAME_LEN + 1;

/// A validated section name, ready to hand to Win32.
#[derive(Clone)]
pub struct SegmentName {
    wide: [u16; WIDE_CAP],
    /// UTF-16 units before the NUL.
    len: usize,
}

impl SegmentName {
    /// Builds a name in the session-local namespace (`Local\<name>`).
    ///
    /// This is the right choice for a feed handler and a consumer started by
    /// the same user in the same session.
    pub fn local(name: &str) -> Result<Self, ShmError> {
        Self::build(LOCAL, name)
    }

    /// Builds a name in the global namespace (`Global\<name>`).
    ///
    /// Use only when producer and consumer live in different sessions (a
    /// service and a desktop process). The **creating** side then needs
    /// `SeCreateGlobalPrivilege`, or `CreateFileMappingW` fails with
    /// `ERROR_PRIVILEGE_NOT_HELD` (1314).
    pub fn global(name: &str) -> Result<Self, ShmError> {
        Self::build(GLOBAL, name)
    }

    fn build(prefix: &str, name: &str) -> Result<Self, ShmError> {
        if name.is_empty() {
            return Err(ShmError::NameEmpty);
        }
        if name.len() > MAX_NAME_LEN {
            return Err(ShmError::NameTooLong { len: name.len() });
        }
        for &b in name.as_bytes() {
            // Printable ASCII only, and no separator: a backslash would push
            // the object into another namespace, which is not something a
            // config string should be able to do by accident.
            if !(0x21..=0x7e).contains(&b) || b == b'\\' || b == b'/' {
                return Err(ShmError::NameChar { byte: b });
            }
        }

        let mut wide = [0u16; WIDE_CAP];
        let mut len = 0;
        for b in prefix.bytes().chain(name.bytes()) {
            wide[len] = b as u16;
            len += 1;
        }
        // `wide[len]` is already 0: the NUL terminator.
        Ok(Self { wide, len })
    }

    /// Pointer to the NUL-terminated UTF-16 name.
    #[inline]
    pub fn as_ptr(&self) -> *const u16 {
        self.wide.as_ptr()
    }
}

impl fmt::Display for SegmentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Every unit is printable ASCII by construction.
        for &u in &self.wide[..self.len] {
            f.write_str(core::str::from_utf8(&[u as u8]).unwrap_or("?"))?;
        }
        Ok(())
    }
}

impl fmt::Debug for SegmentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SegmentName({self})")
    }
}

impl PartialEq for SegmentName {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.wide[..self.len] == other.wide[..other.len]
    }
}

impl Eq for SegmentName {}
