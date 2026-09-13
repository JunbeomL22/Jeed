//! `signal(2)` for `SIGINT` and `SIGTERM`.
//!
//! `signal` rather than `sigaction` because the only thing asked of the
//! handler is to run once and set a flag, and glibc's `signal` already has
//! BSD semantics (the handler stays installed, the interrupted call is
//! restarted).

use super::SignalError;
use core::ffi::c_int;

const SIGINT: c_int = 2;
const SIGTERM: c_int = 15;
const SIG_ERR: usize = usize::MAX;

unsafe extern "C" {
    /// `sighandler_t` in and out. The return is kept as an integer because
    /// the only thing done with it is a comparison against `SIG_ERR`.
    fn signal(signum: c_int, handler: extern "C" fn(c_int)) -> usize;
    fn __errno_location() -> *mut c_int;
}

extern "C" fn handler(_signum: c_int) {
    super::from_handler();
}

pub(super) fn install() -> Result<(), SignalError> {
    for sig in [SIGINT, SIGTERM] {
        // SAFETY: `handler` is async-signal-safe — it performs one atomic
        // store — and has the signature `sighandler_t` names.
        let previous = unsafe { signal(sig, handler) };
        if previous == SIG_ERR {
            // SAFETY: `__errno_location` returns a valid pointer for the life
            // of the thread.
            let code = unsafe { *__errno_location() as u32 };
            return Err(SignalError { call: "signal", code });
        }
    }
    Ok(())
}
