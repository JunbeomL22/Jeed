//! `SetThreadAffinityMask` and `GetLogicalProcessorInformationEx`.
//!
//! Five entry points, declared here rather than taken from a bindings crate —
//! the same choice `jeed-shm` makes, for the same reason.

use super::{CpuError, MAX_LOGICAL, Mask};
use core::ffi::c_void;

const RELATION_PROCESSOR_CORE: u32 = 0;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThread() -> *mut c_void;
    fn SetThreadAffinityMask(thread: *mut c_void, mask: usize) -> usize;
    fn GetCurrentProcessorNumber() -> u32;
    fn GetLogicalProcessorInformationEx(relationship: u32, buffer: *mut u8, length: *mut u32) -> i32;
    fn GetLastError() -> u32;
}

pub(super) fn pin_current_thread(mask: Mask) -> Result<(), CpuError> {
    // SAFETY: `GetCurrentThread` returns a pseudo-handle that is always valid
    // and never needs closing; `SetThreadAffinityMask` reads nothing but its
    // two scalar arguments.
    let previous = unsafe { SetThreadAffinityMask(GetCurrentThread(), mask as usize) };
    if previous == 0 {
        // SAFETY: plain call, no arguments.
        let code = unsafe { GetLastError() };
        return Err(CpuError::Os { call: "SetThreadAffinityMask", code });
    }
    Ok(())
}

pub(super) fn current_processor() -> usize {
    // SAFETY: plain call, no arguments.
    unsafe { GetCurrentProcessorNumber() as usize }
}

/// One `SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX` record per physical core,
/// each carrying the mask of its logical processors.
///
/// ```text
/// offset  0  Relationship  u32
///         4  Size          u32   — bytes in this record, including the header
///         8  Flags         u8    ┐
///         9  EfficiencyClass u8  │ PROCESSOR_RELATIONSHIP
///        10  Reserved      [u8; 20]
///        30  GroupCount    u16   │
///        32  GroupMask     [GROUP_AFFINITY; GroupCount]
///                            0  Mask   usize
///                            8  Group  u16
///                           10  Reserved [u16; 3]      16 bytes each
/// ```
///
/// Read by offset from the byte buffer rather than through a transcribed
/// struct: the records are variable-length and the last field is a flexible
/// array, which is exactly the shape a Rust struct declaration cannot spell.
pub(super) fn siblings() -> Result<Vec<Mask>, CpuError> {
    let mut len: u32 = 0;
    // SAFETY: a null buffer with a zero length is the documented way to ask
    // for the size; the call writes only `len`.
    let ok = unsafe {
        GetLogicalProcessorInformationEx(RELATION_PROCESSOR_CORE, core::ptr::null_mut(), &mut len)
    };
    if ok != 0 {
        return Err(CpuError::NoProcessors);
    }
    // SAFETY: plain call, no arguments.
    let code = unsafe { GetLastError() };
    if code != ERROR_INSUFFICIENT_BUFFER {
        return Err(CpuError::Os { call: "GetLogicalProcessorInformationEx", code });
    }

    let mut buf = vec![0u8; len as usize];
    // SAFETY: `buf` is `len` bytes of writable memory and `len` says so.
    let ok = unsafe {
        GetLogicalProcessorInformationEx(RELATION_PROCESSOR_CORE, buf.as_mut_ptr(), &mut len)
    };
    if ok == 0 {
        // SAFETY: plain call, no arguments.
        let code = unsafe { GetLastError() };
        return Err(CpuError::Os { call: "GetLogicalProcessorInformationEx", code });
    }
    let buf = &buf[..len as usize];

    let mut siblings: Vec<Mask> = Vec::new();
    let mut pos = 0;
    while pos + 32 <= buf.len() {
        let relationship = u32::from_le_bytes(buf[pos..pos + 4].try_into().expect("4 bytes"));
        let size = u32::from_le_bytes(buf[pos + 4..pos + 8].try_into().expect("4 bytes")) as usize;
        if size < 32 || pos + size > buf.len() {
            break;
        }
        if relationship == RELATION_PROCESSOR_CORE {
            let groups = u16::from_le_bytes(buf[pos + 30..pos + 32].try_into().expect("2 bytes"));
            for g in 0..groups as usize {
                let at = pos + 32 + g * 16;
                if at + 16 > pos + size {
                    break;
                }
                let mask = usize::from_le_bytes(buf[at..at + 8].try_into().expect("8 bytes")) as Mask;
                let group = u16::from_le_bytes(buf[at + 8..at + 10].try_into().expect("2 bytes"));
                if group != 0 {
                    // Processor group 1+ is beyond a 64-bit mask, and beyond
                    // this handler.
                    continue;
                }
                for lp in 0..MAX_LOGICAL {
                    if mask & (1 << lp) != 0 {
                        if siblings.len() <= lp {
                            siblings.resize(lp + 1, 0);
                        }
                        siblings[lp] = mask;
                    }
                }
            }
        }
        pos += size;
    }

    // A processor the enumeration skipped would read as sibling-less, which
    // is the safe direction (nothing is forbidden that should be allowed),
    // but say so rather than pretend.
    if siblings.contains(&0) {
        return Err(CpuError::NoProcessors);
    }
    Ok(siblings)
}
