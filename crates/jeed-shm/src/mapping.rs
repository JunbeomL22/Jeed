//! Pagefile-backed named section, mapped into this process.
//!
//! `CreateFileMappingW(INVALID_HANDLE_VALUE, …)` + `MapViewOfFile`
//! (`documents/feed_handler.md` §3). A *file*-backed mapping is deliberately
//! avoided: it drags the cache manager and lazy flushes into the hot path.
//!
//! The six entry points below are declared here rather than taken from a
//! bindings crate — see `Cargo.toml`.

use crate::error::ShmError;
use crate::name::SegmentName;
use core::ffi::c_void;

type Handle = *mut c_void;

const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
const PAGE_READWRITE: u32 = 0x0000_0004;
const FILE_MAP_WRITE: u32 = 0x0000_0002;
const FILE_MAP_READ: u32 = 0x0000_0004;
const ERROR_ALREADY_EXISTS: u32 = 183;

/// Subset of `MEMORY_BASIC_INFORMATION` (x86-64 layout), with the padding the
/// C struct leaves implicit written out.
#[repr(C)]
struct MemoryBasicInformation {
    base_address: *mut c_void,
    allocation_base: *mut c_void,
    allocation_protect: u32,
    partition_id: u16,
    _pad0: u16,
    region_size: usize,
    state: u32,
    protect: u32,
    kind: u32,
    _pad1: u32,
}

impl MemoryBasicInformation {
    const fn zeroed() -> Self {
        Self {
            base_address: core::ptr::null_mut(),
            allocation_base: core::ptr::null_mut(),
            allocation_protect: 0,
            partition_id: 0,
            _pad0: 0,
            region_size: 0,
            state: 0,
            protect: 0,
            kind: 0,
            _pad1: 0,
        }
    }
}

#[cfg(target_arch = "x86_64")]
const _: () = assert!(size_of::<MemoryBasicInformation>() == 48);

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateFileMappingW(
        file: Handle,
        attributes: *const c_void,
        protect: u32,
        max_size_high: u32,
        max_size_low: u32,
        name: *const u16,
    ) -> Handle;

    fn OpenFileMappingW(desired_access: u32, inherit: i32, name: *const u16) -> Handle;

    fn MapViewOfFile(
        mapping: Handle,
        desired_access: u32,
        offset_high: u32,
        offset_low: u32,
        bytes_to_map: usize,
    ) -> *mut c_void;

    fn UnmapViewOfFile(base: *const c_void) -> i32;

    fn CloseHandle(object: Handle) -> i32;

    fn VirtualQuery(
        address: *const c_void,
        buffer: *mut MemoryBasicInformation,
        length: usize,
    ) -> usize;

    fn GetLastError() -> u32;
}

fn last_error(call: &'static str) -> ShmError {
    // SAFETY: no preconditions.
    ShmError::Os { call, code: unsafe { GetLastError() } }
}

/// How a mapping is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Producer: read and write.
    ReadWrite,

    /// Consumer: read only. The OS then enforces what the SPSC discipline
    /// already promises.
    ReadOnly,
}

/// An owned view of a named section.
///
/// The view is unmapped and the section handle closed on drop. The section
/// itself dies when the last handle to it closes, so the segment survives a
/// consumer restart as long as the producer is alive, and vice versa.
pub struct SharedMapping {
    handle: Handle,
    base: *mut u8,
    len: usize,
    existed: bool,
    access: Access,
}

// SAFETY: the mapping is a process-wide resource addressed by a raw pointer;
// moving the owner to another thread (the pinned receive thread) moves the
// only handle with it. It is deliberately not `Sync` — the ring discipline is
// single-producer, single-consumer.
unsafe impl Send for SharedMapping {}

impl SharedMapping {
    /// Creates the section, or opens it if one of that name already exists.
    ///
    /// `len` is the size requested for a **new** section. When a section of
    /// that name already exists its size wins and `len` is only checked
    /// against it — a restarted producer therefore reattaches to the memory it
    /// was using before, which is exactly what `boot_id` is there to signal.
    pub fn create(name: &SegmentName, len: usize) -> Result<Self, ShmError> {
        // SAFETY: `name` is NUL-terminated UTF-16 by construction; passing
        // INVALID_HANDLE_VALUE as the file selects pagefile backing.
        let handle = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                core::ptr::null(),
                PAGE_READWRITE,
                (len >> 32) as u32,
                (len & 0xffff_ffff) as u32,
                name.as_ptr(),
            )
        };
        if handle.is_null() {
            return Err(last_error("CreateFileMappingW"));
        }
        // SAFETY: no preconditions. Read before any other call clobbers it.
        let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;

        Self::map(handle, Access::ReadWrite, existed, len)
    }

    /// Opens an existing section read-only.
    ///
    /// The size is not supplied: the whole section is mapped and its extent
    /// read back from the OS, because the consumer must not take the
    /// producer's word for how much memory there is.
    pub fn open(name: &SegmentName) -> Result<Self, ShmError> {
        // SAFETY: `name` is NUL-terminated UTF-16 by construction.
        let handle = unsafe { OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(last_error("OpenFileMappingW"));
        }
        Self::map(handle, Access::ReadOnly, true, 0)
    }

    fn map(handle: Handle, access: Access, existed: bool, needed: usize) -> Result<Self, ShmError> {
        let rights = match access {
            Access::ReadWrite => FILE_MAP_READ | FILE_MAP_WRITE,
            Access::ReadOnly => FILE_MAP_READ,
        };
        // SAFETY: `handle` is a live section handle. A `bytes_to_map` of 0
        // maps from the offset to the end of the section.
        let base = unsafe { MapViewOfFile(handle, rights, 0, 0, 0) };
        if base.is_null() {
            let err = last_error("MapViewOfFile");
            // SAFETY: `handle` is live and not otherwise owned.
            unsafe { CloseHandle(handle) };
            return Err(err);
        }

        let len = region_size(base);
        if len < needed {
            let err = ShmError::SegmentTooSmall { needed, mapped: len };
            // SAFETY: both are live and owned solely by this frame.
            unsafe {
                UnmapViewOfFile(base);
                CloseHandle(handle);
            }
            return Err(err);
        }

        Ok(Self { handle, base: base.cast::<u8>(), len, existed, access })
    }

    /// `true` if [`create`](Self::create) found an existing section instead of
    /// making one — i.e. this producer is a restart, or a second producer is
    /// running and should not be.
    #[inline]
    pub fn existed(&self) -> bool {
        self.existed
    }

    /// How this view was opened.
    #[inline]
    pub fn access(&self) -> Access {
        self.access
    }

    /// Mapped length in bytes, as reported by the OS (page-rounded up from the
    /// requested size).
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// `true` when nothing is mapped. Present because clippy asks for it; a
    /// live `SharedMapping` always has a non-zero extent.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Base of the mapped view. 64 KiB aligned, so every record slot inside is
    /// cache-line aligned.
    #[inline]
    pub fn as_ptr(&self) -> *mut u8 {
        self.base
    }
}

impl Drop for SharedMapping {
    fn drop(&mut self) {
        // SAFETY: both were produced by this type and are dropped once.
        unsafe {
            UnmapViewOfFile(self.base.cast::<c_void>());
            CloseHandle(self.handle);
        }
    }
}

impl core::fmt::Debug for SharedMapping {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SharedMapping")
            .field("base", &self.base)
            .field("len", &self.len)
            .field("existed", &self.existed)
            .field("access", &self.access)
            .finish()
    }
}

/// Extent of the mapped region starting at `base`, or 0 if the query fails.
fn region_size(base: *mut c_void) -> usize {
    let mut info = MemoryBasicInformation::zeroed();
    // SAFETY: `base` is a live mapped address and `info` is a valid, correctly
    // sized output buffer.
    let written =
        unsafe { VirtualQuery(base, &raw mut info, size_of::<MemoryBasicInformation>()) };
    if written == 0 { 0 } else { info.region_size }
}
