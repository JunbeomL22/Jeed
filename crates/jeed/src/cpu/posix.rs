//! `sched_setaffinity` and sysfs.
//!
//! Three entry points, declared here rather than taken from a bindings crate —
//! the same choice `jeed-shm` makes, for the same reason.

use super::{CpuError, MAX_LOGICAL, Mask};
use core::ffi::c_int;

/// glibc's `cpu_set_t` is 1024 bits.
const CPU_SET_WORDS: usize = 16;

unsafe extern "C" {
    fn sched_setaffinity(pid: c_int, cpusetsize: usize, mask: *const u64) -> c_int;
    fn sched_getcpu() -> c_int;
    fn __errno_location() -> *mut c_int;
}

fn errno() -> u32 {
    // SAFETY: `__errno_location` returns a valid pointer to this thread's
    // errno for the life of the thread.
    unsafe { *__errno_location() as u32 }
}

pub(super) fn pin_current_thread(mask: Mask) -> Result<(), CpuError> {
    let mut set = [0u64; CPU_SET_WORDS];
    set[0] = mask;
    // SAFETY: `set` is a correctly sized `cpu_set_t`; pid 0 is the calling
    // thread.
    let rc = unsafe { sched_setaffinity(0, size_of_val(&set), set.as_ptr()) };
    if rc != 0 {
        return Err(CpuError::Os { call: "sched_setaffinity", code: errno() });
    }
    Ok(())
}

pub(super) fn current_processor() -> usize {
    // SAFETY: plain call, no arguments.
    let cpu = unsafe { sched_getcpu() };
    cpu.max(0) as usize
}

/// `/sys/devices/system/cpu/cpuN/topology/thread_siblings_list`, one file per
/// logical processor, in `0-1` / `2,10` list form.
pub(super) fn siblings() -> Result<Vec<Mask>, CpuError> {
    let mut out = Vec::new();
    for lp in 0..MAX_LOGICAL {
        let path = format!("/sys/devices/system/cpu/cpu{lp}/topology/thread_siblings_list");
        let Ok(text) = std::fs::read_to_string(&path) else {
            break;
        };
        out.push(parse_list(text.trim())?);
    }
    Ok(out)
}

/// `0-1,4,6-7` → mask.
fn parse_list(s: &str) -> Result<Mask, CpuError> {
    let mut mask = 0;
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (lo, hi) = match part.split_once('-') {
            Some((a, b)) => (a.parse::<usize>(), b.parse::<usize>()),
            None => (part.parse::<usize>(), part.parse::<usize>()),
        };
        let (Ok(lo), Ok(hi)) = (lo, hi) else {
            return Err(CpuError::NoProcessors);
        };
        for lp in lo..=hi {
            if lp >= MAX_LOGICAL {
                return Err(CpuError::BeyondMask);
            }
            mask |= 1 << lp;
        }
    }
    Ok(mask)
}
