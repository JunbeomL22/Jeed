//! `/dev/shm` object, mapped with `mmap`.
//!
//! `shm_open` + `ftruncate` + `mmap`, the POSIX counterpart of the Windows
//! pagefile section. The eight entry points below are declared here rather than
//! taken from a bindings crate — see `Cargo.toml`.
//!
//! ## The object outlives the process
//!
//! Unlike a Windows section, this one is a name in a filesystem: it survives
//! every process that mapped it until [`unlink`] removes it. That is why
//! `existed` is weaker evidence here (`mapping` module docs) and why nothing in
//! this module unlinks on drop — a live consumer may still be reading, and
//! whether a stale segment should be cleared is an operational decision.
//!
//! ## The extent comes from `lseek`, not `fstat`
//!
//! `struct stat` has a different shape on every architecture and libc, and
//! getting one field out of it is not worth transcribing. A shared-memory
//! object is a regular file on `tmpfs`, so seeking to its end reports the same
//! size with one portable call and no struct at all.

use super::{Access, Mapped};
use crate::error::ShmError;
use crate::name::SegmentName;
use core::ffi::{c_char, c_int, c_void};

const O_RDONLY: c_int = 0;
const O_RDWR: c_int = 0o2;
const O_CREAT: c_int = 0o100;
const O_EXCL: c_int = 0o200;

const PROT_READ: c_int = 1;
const PROT_WRITE: c_int = 2;
const MAP_SHARED: c_int = 1;
const MAP_FAILED: *mut c_void = usize::MAX as *mut c_void;

const SEEK_END: c_int = 2;

const EEXIST: i32 = 17;

/// Owner and group read/write, nobody else. Producer and consumer are the same
/// user; a segment readable by the rest of the box is not.
const MODE: u32 = 0o660;

/// The descriptor the object was mapped from.
///
/// A mapping keeps the object alive on its own, so this could be closed right
/// after `mmap`. It is held for the life of the mapping instead, so that
/// release is one place on both platforms — Windows closes a handle here too.
pub(super) struct Owner {
    fd: c_int,
}

unsafe extern "C" {
    fn shm_open(name: *const c_char, oflag: c_int, mode: u32) -> c_int;
    fn shm_unlink(name: *const c_char) -> c_int;
    fn ftruncate(fd: c_int, length: i64) -> c_int;
    fn lseek(fd: c_int, offset: i64, whence: c_int) -> i64;
    fn close(fd: c_int) -> c_int;
    fn mmap(
        addr: *mut c_void,
        length: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: i64,
    ) -> *mut c_void;
    fn munmap(addr: *mut c_void, length: usize) -> c_int;
    fn __errno_location() -> *mut i32;
}

fn errno() -> i32 {
    // SAFETY: glibc and musl both return a pointer to this thread's `errno`,
    // which is live for the life of the thread.
    unsafe { *__errno_location() }
}

fn last_error(call: &'static str) -> ShmError {
    ShmError::Os { call, code: errno() as u32 }
}

pub(super) fn create(name: &SegmentName, len: usize) -> Result<Mapped, ShmError> {
    // Exclusive first, so that "I made this" and "it was already there" are
    // distinguished by the kernel rather than by a racy existence check.
    // SAFETY: `name` is NUL-terminated ASCII with a single leading `/`.
    let mut fd = unsafe { shm_open(name.as_ptr(), O_RDWR | O_CREAT | O_EXCL, MODE) };
    let existed = fd < 0 && errno() == EEXIST;
    if existed {
        // SAFETY: as above.
        fd = unsafe { shm_open(name.as_ptr(), O_RDWR, MODE) };
    }
    if fd < 0 {
        return Err(last_error("shm_open"));
    }

    // A fresh object is zero length until it is given one. An existing one is
    // left at the size it has, so a restarted producer reattaches to the memory
    // it was already using — `boot_id` is what tells the consumer.
    if !existed && unsafe { ftruncate(fd, len as i64) } < 0 {
        let err = last_error("ftruncate");
        // SAFETY: `fd` is live and owned by this frame.
        unsafe { close(fd) };
        return Err(err);
    }

    map(fd, PROT_READ | PROT_WRITE, existed, len)
}

pub(super) fn open(name: &SegmentName, access: Access) -> Result<Mapped, ShmError> {
    let (oflag, prot) = match access {
        Access::ReadWrite => (O_RDWR, PROT_READ | PROT_WRITE),
        Access::ReadOnly => (O_RDONLY, PROT_READ),
    };
    // SAFETY: `name` is NUL-terminated ASCII with a single leading `/`.
    let fd = unsafe { shm_open(name.as_ptr(), oflag, 0) };
    if fd < 0 {
        return Err(last_error("shm_open"));
    }
    map(fd, prot, true, 0)
}

pub(super) fn unlink(name: &SegmentName) -> Result<(), ShmError> {
    // SAFETY: `name` is NUL-terminated ASCII.
    if unsafe { shm_unlink(name.as_ptr()) } < 0 {
        return Err(last_error("shm_unlink"));
    }
    Ok(())
}

/// # Safety
///
/// `m` must have come from [`create`] or [`open`] and be released once.
pub(super) unsafe fn release(m: &Mapped) {
    // SAFETY: the range was returned by `mmap` in this module and the
    // descriptor by `shm_open`; both are released once.
    unsafe {
        munmap(m.base.cast::<c_void>(), m.len);
        close(m.owner.fd);
    }
}

/// Maps the whole object.
fn map(fd: c_int, prot: c_int, existed: bool, needed: usize) -> Result<Mapped, ShmError> {
    // SAFETY: `fd` is a live descriptor on a shared-memory object.
    let len = unsafe { lseek(fd, 0, SEEK_END) };
    if len < 0 {
        let err = last_error("lseek");
        // SAFETY: `fd` is live and owned by this frame.
        unsafe { close(fd) };
        return Err(err);
    }
    let len = len as usize;

    if len < needed {
        // SAFETY: as above.
        unsafe { close(fd) };
        return Err(ShmError::SegmentTooSmall { needed, mapped: len });
    }

    // SAFETY: `fd` is live, `len` is its full extent, and a null hint lets the
    // kernel choose the address.
    let base = unsafe { mmap(core::ptr::null_mut(), len, prot, MAP_SHARED, fd, 0) };
    if base == MAP_FAILED || base.is_null() {
        let err = last_error("mmap");
        // SAFETY: `fd` is live and owned by this frame.
        unsafe { close(fd) };
        return Err(err);
    }
    Ok(Mapped { base: base.cast::<u8>(), len, existed, owner: Owner { fd } })
}
