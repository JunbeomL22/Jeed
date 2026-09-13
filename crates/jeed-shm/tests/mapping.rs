//! `jeed_shm::mapping` — the Win32 section underneath the ring.

mod common;

use common::unique;
use jeed_shm::{Access, SegmentName, SharedMapping, ShmError};

/// `ERROR_FILE_NOT_FOUND`.
const ERROR_FILE_NOT_FOUND: u32 = 2;

#[test]
fn create_then_open_addresses_the_same_memory() {
    let name = unique("same");
    let w = SharedMapping::create(&name, 4096).unwrap();
    let r = SharedMapping::open(&name).unwrap();

    // SAFETY: both views cover at least 4096 writable/readable bytes.
    unsafe {
        w.as_ptr().write(0xab);
        w.as_ptr().add(4095).write(0xcd);
        assert_eq!(r.as_ptr().read(), 0xab);
        assert_eq!(r.as_ptr().add(4095).read(), 0xcd);
    }
    assert_ne!(w.as_ptr(), r.as_ptr(), "two views, one section");
}

#[test]
fn access_is_recorded_and_differs_by_end() {
    let name = unique("access");
    let w = SharedMapping::create(&name, 4096).unwrap();
    let r = SharedMapping::open(&name).unwrap();
    assert_eq!(w.access(), Access::ReadWrite);
    assert_eq!(r.access(), Access::ReadOnly);
}

#[test]
fn the_mapped_extent_is_at_least_what_was_asked_for() {
    let name = unique("extent");
    let m = SharedMapping::create(&name, 4096 + 1).unwrap();
    assert!(m.len() >= 4097, "{}", m.len());
    assert!(!m.is_empty());
}

#[test]
fn opening_a_segment_nobody_created_is_an_os_error_not_a_panic() {
    let name = unique("absent");
    assert_eq!(
        SharedMapping::open(&name).unwrap_err(),
        ShmError::Os { call: "OpenFileMappingW", code: ERROR_FILE_NOT_FOUND }
    );
}

#[test]
fn a_second_create_finds_the_first_section() {
    let name = unique("twice");
    let first = SharedMapping::create(&name, 4096).unwrap();
    assert!(!first.existed(), "nothing of this name existed");

    let second = SharedMapping::create(&name, 4096).unwrap();
    assert!(second.existed(), "a live producer is already on this name");
}

#[test]
fn the_section_dies_with_its_last_handle() {
    let name = unique("lifetime");
    {
        let _keep = SharedMapping::create(&name, 4096).unwrap();
        assert!(SharedMapping::open(&name).is_ok());
    }
    // The consumer holding it open is what keeps a segment alive across a
    // producer restart; with nobody holding it, it is gone.
    assert!(SharedMapping::open(&name).is_err());
}

#[test]
fn a_consumer_outlives_the_producer_that_made_the_section() {
    let name = unique("outlive");
    let reader = {
        let w = SharedMapping::create(&name, 4096).unwrap();
        // SAFETY: 4096 writable bytes.
        unsafe { w.as_ptr().write(0x5a) };
        SharedMapping::open(&name).unwrap()
    };
    // SAFETY: the section is still alive because `reader` holds a handle.
    assert_eq!(unsafe { reader.as_ptr().read() }, 0x5a);
}

#[test]
fn an_impossible_size_fails_at_create_time() {
    // 2^47 bytes: the section object cannot be committed against the pagefile.
    let name = unique("huge");
    assert!(matches!(
        SharedMapping::create(&name, 1 << 47),
        Err(ShmError::Os { call: "CreateFileMappingW", .. })
    ));
}

#[test]
fn a_mapping_can_be_moved_to_the_receive_thread() {
    let name = SegmentName::local(&format!("jeed.test.{}.send", std::process::id())).unwrap();
    let m = SharedMapping::create(&name, 4096).unwrap();
    let handle = std::thread::spawn(move || {
        // SAFETY: 4096 writable bytes, and this thread now owns the mapping.
        unsafe { m.as_ptr().write(0x11) };
        m.len()
    });
    assert!(handle.join().unwrap() >= 4096);
}
