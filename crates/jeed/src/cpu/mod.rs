//! Core pinning and the SMT topology it has to respect.
//!
//! ```text
//! conf  cores = [2]        ──→  mask 0b100  ──→  SetThreadAffinityMask / sched_setaffinity
//!       cores = [4,5,6,7]  ──→  mask 0xF0
//!
//! topology   core 2 ⇄ logical 2 and 10 (SMT pair)   ──→  nothing else may use 10
//! ```
//!
//! ## Pinning is not isolation
//!
//! `SetThreadAffinityMask` confines *this* thread to the mask; it does not
//! keep anything else off those processors, and Windows has no `isolcpus`
//! (`documents/feed_handler.md` §14). The best that can be done from a conf
//! is to keep the handler's own threads apart: a spinning thread on one
//! logical processor, and its SMT sibling left empty — a sibling running the
//! cold feed would share the core's execution units and halve the spin. That
//! is what [`Topology`] is for, and it is a start-up check, not a runtime
//! one.
//!
//! ## Sixty-four
//!
//! Masks are `u64`, one bit per logical processor, so processor 64 and up are
//! out of reach. On Windows that is the boundary of processor group 0 as well,
//! and crossing it would mean `SetThreadGroupAffinity`. The box this runs on
//! has sixteen; a 65th processor is a different deployment.

#[cfg_attr(windows, path = "windows.rs")]
#[cfg_attr(unix, path = "posix.rs")]
mod imp;

use core::fmt;

/// Highest logical processor a mask can name, plus one.
pub const MAX_LOGICAL: usize = 64;

/// One bit per logical processor.
pub type Mask = u64;

/// The mask naming `cores`, or `None` if any of them is out of range.
pub fn mask_of(cores: &[u16]) -> Option<Mask> {
    let mut mask = 0;
    for &c in cores {
        if c as usize >= MAX_LOGICAL {
            return None;
        }
        mask |= 1 << c;
    }
    Some(mask)
}

/// Confines the calling thread to the processors in `mask`.
///
/// Fails on an empty mask, and on a mask naming a processor the OS does not
/// have — the OS reports the latter, and the error names the call.
pub fn pin_current_thread(mask: Mask) -> Result<(), CpuError> {
    if mask == 0 {
        return Err(CpuError::EmptyMask);
    }
    imp::pin_current_thread(mask)
}

/// The logical processor the calling thread is on right now.
///
/// Advisory: the answer can be stale by the time it is read, unless the
/// thread is pinned to exactly one processor — which is the case this is
/// used to verify.
pub fn current_processor() -> usize {
    imp::current_processor()
}

/// Which logical processors share a physical core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Topology {
    /// `siblings[i]` is the mask of every logical processor on the same core
    /// as `i`, including `i` itself.
    siblings: Vec<Mask>,
}

impl Topology {
    /// Asks the OS.
    ///
    /// `GetLogicalProcessorInformationEx(RelationProcessorCore)` on Windows,
    /// `/sys/devices/system/cpu/cpuN/topology/thread_siblings_list` on Linux.
    pub fn detect() -> Result<Self, CpuError> {
        let siblings = imp::siblings()?;
        if siblings.is_empty() {
            return Err(CpuError::NoProcessors);
        }
        Ok(Self { siblings })
    }

    /// A topology stated rather than detected — for tests, and for validating
    /// a conf against the box it will run on rather than the one it is read
    /// on.
    pub fn from_siblings(siblings: Vec<Mask>) -> Self {
        Self { siblings }
    }

    /// Number of logical processors.
    #[inline]
    pub fn logical(&self) -> usize {
        self.siblings.len()
    }

    /// Mask of the processors sharing a core with `lp`, `lp` included; `None`
    /// if `lp` is not a processor this topology knows.
    #[inline]
    pub fn siblings_of(&self, lp: usize) -> Option<Mask> {
        self.siblings.get(lp).copied()
    }

    /// Mask of every processor sharing a core with any processor in `mask`,
    /// **excluding** the processors in `mask` themselves.
    pub fn siblings_outside(&self, mask: Mask) -> Mask {
        let mut out = 0;
        for lp in 0..self.siblings.len().min(MAX_LOGICAL) {
            if mask & (1 << lp) != 0 {
                out |= self.siblings[lp];
            }
        }
        out & !mask
    }
}

/// Why a pin or a topology query failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuError {
    /// A mask with no bits set.
    EmptyMask,

    /// The OS reported no processors, which it never does.
    NoProcessors,

    /// A processor beyond [`MAX_LOGICAL`], or in a processor group other than
    /// the first.
    BeyondMask,

    /// An OS call failed.
    Os {
        /// The call, by its platform name.
        call: &'static str,
        /// `GetLastError()` on Windows, `errno` on POSIX.
        code: u32,
    },
}

impl fmt::Display for CpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyMask => write!(f, "affinity mask names no processor"),
            Self::NoProcessors => write!(f, "the OS reported no processors"),
            Self::BeyondMask => write!(f, "a processor beyond {MAX_LOGICAL} cannot be named"),
            Self::Os { call, code } => write!(f, "{call} failed with code {code}"),
        }
    }
}

impl std::error::Error for CpuError {}
