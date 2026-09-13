//! `jeed::signal` — the stop flag.
//!
//! The flag is process-wide, so this is one test: a second one could not know
//! whether the flag it reads was set by itself.

use jeed::signal;

#[test]
fn the_flag_starts_clear_and_a_request_sets_it() {
    signal::install().unwrap();
    signal::install().unwrap();
    assert!(!signal::requested(), "nothing has asked yet");
    signal::request();
    assert!(signal::requested());
    assert!(signal::requested(), "and it stays set");
}
