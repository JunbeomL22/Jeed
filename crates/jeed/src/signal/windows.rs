//! `SetConsoleCtrlHandler`.

use super::SignalError;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetConsoleCtrlHandler(handler: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32;
    fn GetLastError() -> u32;
}

/// Runs on a console-owned thread, not on any of ours. Sets the flag and
/// reports the event handled; Windows then leaves the process alone (for
/// Ctrl-C and Ctrl-Break) or kills it after a grace period (for close,
/// logoff and shutdown), and either way the receive loops have been told.
unsafe extern "system" fn handler(_ctrl_type: u32) -> i32 {
    super::from_handler();
    1
}

pub(super) fn install() -> Result<(), SignalError> {
    // SAFETY: `handler` has the signature the console expects and touches
    // only an atomic.
    let ok = unsafe { SetConsoleCtrlHandler(Some(handler), 1) };
    if ok == 0 {
        // SAFETY: plain call, no arguments.
        let code = unsafe { GetLastError() };
        return Err(SignalError { call: "SetConsoleCtrlHandler", code });
    }
    Ok(())
}
