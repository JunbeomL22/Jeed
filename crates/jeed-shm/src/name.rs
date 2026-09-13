//! Validated segment names.
//!
//! A name is an OS object name, not a path. It is built once at startup and
//! held in an inline, NUL-terminated buffer in whatever encoding the platform
//! wants, so that opening or re-opening a segment never allocates.
//!
//! ```text
//! SegmentName::local("jeed.krx.hot")
//!   Windows   Local\jeed.krx.hot     (UTF-16, session namespace)
//!   POSIX     /jeed.krx.hot          (ASCII, one flat namespace)
//! ```
//!
//! ## The two namespaces are one namespace on POSIX
//!
//! Windows has a session-local and a global kernel object namespace, and the
//! same text in each names a **different** object. POSIX shared memory has one
//! namespace, scoped by filesystem permissions on `/dev/shm` rather than by a
//! prefix — so [`local`](SegmentName::local) and [`global`](SegmentName::global)
//! produce the same object there, and compare equal.
//!
//! The constructors are kept on both platforms anyway. A handler that names its
//! segment `global` is saying "producer and consumer may live in different
//! sessions", which is a real statement about deployment; Windows enforces it
//! with a namespace and POSIX enforces it with file permissions.

use crate::error::ShmError;
use core::fmt;

/// Longest name accepted, excluding the platform prefix.
///
/// POSIX allows 255 bytes including the leading `/`, so this leaves room for
/// the Windows `Global\` prefix and stays inside both.
pub const MAX_NAME_LEN: usize = 200;

/// Which namespace a name lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Namespace {
    Local,
    Global,
}

#[cfg(windows)]
mod encoding {
    /// `Global\` — the longer of the two prefixes.
    pub const CAP: usize = 7 + super::MAX_NAME_LEN + 1;
    pub const LOCAL: &str = "Local\\";
    pub const GLOBAL: &str = "Global\\";
    /// UTF-16, because that is what the `…W` entry points take.
    pub type Unit = u16;
}

#[cfg(unix)]
mod encoding {
    /// One `/`, the name, and the NUL.
    pub const CAP: usize = 1 + super::MAX_NAME_LEN + 1;
    /// `shm_open` requires a leading slash and no other one.
    pub const LOCAL: &str = "/";
    /// The same object: POSIX has one namespace (see the module docs).
    pub const GLOBAL: &str = "/";
    pub type Unit = core::ffi::c_char;
}

/// A validated segment name, ready to hand to the OS.
#[derive(Clone)]
pub struct SegmentName {
    buf: [encoding::Unit; encoding::CAP],
    /// Units before the NUL.
    len: usize,
}

impl SegmentName {
    /// Builds a name in the session-local namespace.
    ///
    /// This is the right choice for a feed handler and a consumer started by
    /// the same user in the same session.
    pub fn local(name: &str) -> Result<Self, ShmError> {
        Self::build(Namespace::Local, name)
    }

    /// Builds a name in the global namespace.
    ///
    /// Use only when producer and consumer live in different sessions (a
    /// service and a desktop process). On Windows the **creating** side then
    /// needs `SeCreateGlobalPrivilege`, or `CreateFileMappingW` fails with
    /// `ERROR_PRIVILEGE_NOT_HELD` (1314). On POSIX there is no privilege to
    /// hold and no separate namespace — see the module docs.
    pub fn global(name: &str) -> Result<Self, ShmError> {
        Self::build(Namespace::Global, name)
    }

    fn build(namespace: Namespace, name: &str) -> Result<Self, ShmError> {
        if name.is_empty() {
            return Err(ShmError::NameEmpty);
        }
        if name.len() > MAX_NAME_LEN {
            return Err(ShmError::NameTooLong { len: name.len() });
        }
        for &b in name.as_bytes() {
            // Printable ASCII only, and no separator: a backslash would push
            // the object into another Windows namespace and a slash would make
            // a POSIX name the kernel refuses, neither of which is something a
            // config string should be able to do by accident.
            if !(0x21..=0x7e).contains(&b) || b == b'\\' || b == b'/' {
                return Err(ShmError::NameChar { byte: b });
            }
        }

        let prefix = match namespace {
            Namespace::Local => encoding::LOCAL,
            Namespace::Global => encoding::GLOBAL,
        };

        let mut buf = [0 as encoding::Unit; encoding::CAP];
        let mut len = 0;
        for b in prefix.bytes().chain(name.bytes()) {
            buf[len] = b as encoding::Unit;
            len += 1;
        }
        // `buf[len]` is already 0: the NUL terminator.
        Ok(Self { buf, len })
    }

    /// Pointer to the NUL-terminated name, in the platform's encoding —
    /// UTF-16 on Windows, bytes on POSIX.
    #[inline]
    pub fn as_ptr(&self) -> *const encoding::Unit {
        self.buf.as_ptr()
    }
}

impl fmt::Display for SegmentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Every unit is printable ASCII by construction.
        for &u in &self.buf[..self.len] {
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

/// Equality is over the **rendered** name, which is what the OS resolves.
///
/// That is why `local("x") != global("x")` on Windows and `==` on POSIX: the
/// question being answered is "is this the same object?", and the two platforms
/// genuinely disagree.
impl PartialEq for SegmentName {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.buf[..self.len] == other.buf[..other.len]
    }
}

impl Eq for SegmentName {}
