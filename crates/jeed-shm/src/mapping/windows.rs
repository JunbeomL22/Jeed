//! Pagefile-backed named section.
//!
//! `CreateFileMappingW(INVALID_HANDLE_VALUE, …)` + `MapViewOfFile`. The six
//! entry points below are declared here rather than taken from a bindings
//! crate — see `Cargo.toml`.

use super::{Access, Mapped};
use crate::error::ShmError;
use crate::name::SegmentName;
use core::ffi::c_void;

type Handle = *mut c_void;

const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
const PAGE_READWRITE: u32 = 0x0000_0004;
const FILE_MAP_WRITE: u32 = 0x0000_0002;
const FILE_MAP_READ: u32 = 0x0000_0004;
const ERROR_ALREADY_EXISTS: u32 = 183;

/// The section handle, which is what keeps the object alive.
pub(super) struct Owner {
    handle: Handle,
}

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

pub(super) fn create(name: &SegmentName, len: usize) -> Result<Mapped, ShmError> {
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

    map(handle, Access::ReadWrite, existed, len)
}

pub(super) fn open(name: &SegmentName, access: Access) -> Result<Mapped, ShmError> {
    // SAFETY: `name` is NUL-terminated UTF-16 by construction.
    let handle = unsafe { OpenFileMappingW(rights(access), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(last_error("OpenFileMappingW"));
    }
    map(handle, access, true, 0)
}

/// `FILE_MAP_*` bits for an access mode — asked for on the handle and again on
/// the view, which must not exceed it.
const fn rights(access: Access) -> u32 {
    match access {
        Access::ReadWrite => FILE_MAP_READ | FILE_MAP_WRITE,
        Access::ReadOnly => FILE_MAP_READ,
    }
}

/// A section has no name once its handles are gone, so there is nothing to
/// remove. Present so callers can be written once.
pub(super) fn unlink(_name: &SegmentName) -> Result<(), ShmError> {
    Ok(())
}

/// # Safety
///
/// `m` must have come from [`create`] or [`open`] and be released once.
pub(super) unsafe fn release(m: &Mapped) {
    // SAFETY: both were produced by this module and are dropped once.
    unsafe {
        UnmapViewOfFile(m.base.cast::<c_void>());
        CloseHandle(m.owner.handle);
    }
}

fn map(handle: Handle, access: Access, existed: bool, needed: usize) -> Result<Mapped, ShmError> {
    // SAFETY: `handle` is a live section handle. A `bytes_to_map` of 0 maps
    // from the offset to the end of the section.
    let base = unsafe { MapViewOfFile(handle, rights(access), 0, 0, 0) };
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

    Ok(Mapped { base: base.cast::<u8>(), len, existed, owner: Owner { handle } })
}

/// Extent of the mapped region starting at `base`, or 0 if the query fails.
fn region_size(base: *mut c_void) -> usize {
    let mut info = MemoryBasicInformation::zeroed();
    // SAFETY: `base` is a live mapped address and `info` is a valid, correctly
    // sized output buffer.
    let written = unsafe { VirtualQuery(base, &raw mut info, size_of::<MemoryBasicInformation>()) };
    if written == 0 { 0 } else { info.region_size }
}
