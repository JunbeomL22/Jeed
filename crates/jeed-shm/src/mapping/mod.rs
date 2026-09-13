//! The named shared-memory object underneath the ring.
//!
//! | | Windows | POSIX |
//! |---|---|---|
//! | object | pagefile-backed section | `/dev/shm` object |
//! | create | `CreateFileMappingW(INVALID_HANDLE_VALUE, …)` | `shm_open` + `ftruncate` |
//! | map | `MapViewOfFile` | `mmap` |
//! | extent | `VirtualQuery` | `fstat` |
//!
//! A *file*-backed mapping is deliberately avoided on both: it drags the page
//! cache and lazy flushes into the hot path (`documents/feed_handler.md` §3).
//! The entry points are declared inline rather than taken from a bindings
//! crate — see `Cargo.toml`.
//!
//! [`ring`](crate::ring), [`producer`](crate::producer) and
//! [`consumer`](crate::consumer) sit entirely above this module and contain no
//! platform code at all.
//!
//! ## ⚠️ The lifetime rule is the one thing that is not the same
//!
//! **Windows:** the section dies when the last handle to it closes. A segment
//! therefore survives a producer restart exactly as long as a consumer is
//! holding it open, and [`existed`](SharedMapping::existed) at a cold start
//! means a second producer is running.
//!
//! **POSIX:** the object is a name in `/dev/shm` and outlives every process
//! that touched it until someone calls [`unlink`](SharedMapping::unlink). So
//! `existed()` there can equally mean *last week's run left a file*, and it is
//! **not** evidence of a second producer.
//!
//! Nothing above this module depends on the difference, because the restart
//! signal that matters is `boot_id`: a consumer that stayed attached across a
//! restart sees it change and resets
//! ([`SegmentHeader`](jeed_wire::SegmentHeader)). `existed()` stays a
//! diagnostic, and its documentation says which platform it is speaking about.

#[cfg_attr(windows, path = "windows.rs")]
#[cfg_attr(unix, path = "posix.rs")]
mod imp;

use crate::error::ShmError;
use crate::name::SegmentName;

/// How a mapping is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Producer: read and write.
    ReadWrite,

    /// Consumer: read only. The OS then enforces what the SPSC discipline
    /// already promises.
    ReadOnly,
}

/// What a platform hands back from a create or an open.
pub(crate) struct Mapped {
    base: *mut u8,
    len: usize,
    existed: bool,
    owner: imp::Owner,
}

/// An owned view of a named shared-memory object.
///
/// The view is released on drop. What that means for the object itself differs
/// by platform — see the module docs.
pub struct SharedMapping {
    inner: Mapped,
    access: Access,
}

// SAFETY: the mapping is a process-wide resource addressed by a raw pointer;
// moving the owner to another thread (the pinned receive thread) moves the
// only handle with it. It is deliberately not `Sync` — the ring discipline is
// single-producer, single-consumer.
unsafe impl Send for SharedMapping {}

impl SharedMapping {
    /// Creates the object, or opens it if one of that name already exists.
    ///
    /// `len` is the size requested for a **new** object. When one of that name
    /// already exists its size wins and `len` is only checked against it — a
    /// restarted producer therefore reattaches to the memory it was using
    /// before, which is exactly what `boot_id` is there to signal.
    pub fn create(name: &SegmentName, len: usize) -> Result<Self, ShmError> {
        Ok(Self { inner: imp::create(name, len)?, access: Access::ReadWrite })
    }

    /// Opens an existing object read-only.
    ///
    /// The size is not supplied: the whole object is mapped and its extent read
    /// back from the OS, because the consumer must not take the producer's word
    /// for how much memory there is.
    pub fn open(name: &SegmentName) -> Result<Self, ShmError> {
        Ok(Self { inner: imp::open(name)?, access: Access::ReadOnly })
    }

    /// Removes the name, so nothing can attach to it again.
    ///
    /// **POSIX only, and a no-op on Windows**, where a section has no name to
    /// remove once its handles are gone. Existing mappings stay valid; this
    /// only takes the name out of `/dev/shm`.
    ///
    /// Not called from anywhere in this crate. Clearing a segment is an
    /// operational decision — a live consumer may still be reading it — so it
    /// belongs to whoever is doing the clearing, not to a `Drop`.
    pub fn unlink(name: &SegmentName) -> Result<(), ShmError> {
        imp::unlink(name)
    }

    /// `true` if [`create`](Self::create) found an existing object instead of
    /// making one.
    ///
    /// **Read this per platform.** On Windows it means this producer is a
    /// restart, or a second producer is running and should not be. On POSIX the
    /// name outlives every process that used it, so it may equally be a file
    /// left by a run that ended days ago — see the module docs.
    #[inline]
    pub fn existed(&self) -> bool {
        self.inner.existed
    }

    /// How this view was opened.
    #[inline]
    pub fn access(&self) -> Access {
        self.access
    }

    /// Mapped length in bytes, as reported by the OS. At least what was asked
    /// for, and page-rounded up on platforms that round.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len
    }

    /// `true` when nothing is mapped. Present because clippy asks for it; a
    /// live `SharedMapping` always has a non-zero extent.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.len == 0
    }

    /// Base of the mapped view.
    ///
    /// Page aligned at worst, so every record slot inside is cache-line
    /// aligned — which is the property [`ring`](crate::ring) depends on.
    #[inline]
    pub fn as_ptr(&self) -> *mut u8 {
        self.inner.base
    }
}

impl Drop for SharedMapping {
    fn drop(&mut self) {
        // SAFETY: the mapping was produced by `imp` and is released once.
        unsafe { imp::release(&self.inner) };
    }
}

impl core::fmt::Debug for SharedMapping {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SharedMapping")
            .field("base", &self.inner.base)
            .field("len", &self.inner.len)
            .field("existed", &self.inner.existed)
            .field("access", &self.access)
            .finish()
    }
}
