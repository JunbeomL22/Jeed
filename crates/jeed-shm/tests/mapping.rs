//! `jeed_shm::mapping` — the named shared-memory object underneath the ring.

mod common;

use common::unique;
use jeed_shm::{Access, SharedMapping, ShmError};
#[cfg(windows)]
use jeed_shm::SegmentName;

/// Windows `ERROR_FILE_NOT_FOUND` and POSIX `ENOENT` are both 2. The *call*
/// names differ, which is the point of carrying it.
const NOT_FOUND: u32 = 2;

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
    assert_ne!(w.as_ptr(), r.as_ptr(), "two views, one object");
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
fn a_second_create_finds_the_first_object() {
    let name = unique("twice");
    let first = SharedMapping::create(&name, 4096).unwrap();
    assert!(!first.existed(), "nothing of this name existed");

    let second = SharedMapping::create(&name, 4096).unwrap();
    assert!(second.existed(), "the name is already taken");
}

#[test]
fn a_consumer_outlives_the_producer_that_made_the_object() {
    let name = unique("outlive");
    let reader = {
        let w = SharedMapping::create(&name, 4096).unwrap();
        // SAFETY: 4096 writable bytes.
        unsafe { w.as_ptr().write(0x5a) };
        SharedMapping::open(&name).unwrap()
    };
    // SAFETY: the mapping is still live — on Windows because `reader` holds a
    // handle, on POSIX because a mapping keeps the object alive on its own.
    assert_eq!(unsafe { reader.as_ptr().read() }, 0x5a);
}

#[test]
fn a_mapping_can_be_moved_to_the_receive_thread() {
    let name = unique("send");
    let m = SharedMapping::create(&name, 4096).unwrap();
    let handle = std::thread::spawn(move || {
        // SAFETY: 4096 writable bytes, and this thread now owns the mapping.
        unsafe { m.as_ptr().write(0x11) };
        m.len()
    });
    assert!(handle.join().unwrap() >= 4096);
}

// ── where the platforms genuinely differ ────────────────────────────────────

#[cfg(windows)]
#[test]
fn opening_a_segment_nobody_created_names_the_call_that_failed() {
    let name = unique("absent");
    assert_eq!(
        SharedMapping::open(&name).unwrap_err(),
        ShmError::Os { call: "OpenFileMappingW", code: NOT_FOUND }
    );
}

#[cfg(unix)]
#[test]
fn opening_a_segment_nobody_created_names_the_call_that_failed() {
    let name = unique("absent");
    assert_eq!(
        SharedMapping::open(&name).unwrap_err(),
        ShmError::Os { call: "shm_open", code: NOT_FOUND }
    );
}

#[cfg(windows)]
#[test]
fn a_windows_section_dies_with_its_last_handle() {
    let name = unique("lifetime");
    {
        let _keep = SharedMapping::create(&name, 4096).unwrap();
        assert!(SharedMapping::open(&name).is_ok());
    }
    // A consumer holding it open is what keeps a segment alive across a
    // producer restart; with nobody holding it, it is gone.
    assert!(SharedMapping::open(&name).is_err());
}

#[cfg(unix)]
#[test]
fn a_posix_object_outlives_every_process_until_it_is_unlinked() {
    // The other half of the lifetime rule, and the reason `existed()` is weaker
    // evidence here: a name in `/dev/shm` is still there long after everything
    // that mapped it has gone.
    let name = unique("lifetime");
    {
        let _keep = SharedMapping::create(&name, 4096).unwrap();
        assert!(SharedMapping::open(&name).is_ok());
    }
    assert!(SharedMapping::open(&name).is_ok(), "the name survived the mapping");

    SharedMapping::unlink(&name).unwrap();
    assert!(SharedMapping::open(&name).is_err(), "and only unlink removes it");
}

#[cfg(unix)]
#[test]
fn unlinking_a_name_nobody_created_is_an_error_not_a_panic() {
    let name = unique("never");
    assert_eq!(
        SharedMapping::unlink(&name).unwrap_err(),
        ShmError::Os { call: "shm_unlink", code: NOT_FOUND }
    );
}

#[cfg(windows)]
#[test]
fn unlink_is_a_no_op_where_there_is_no_name_to_remove() {
    // A section has no name once its handles are gone, so the call exists only
    // so that callers can be written once.
    let name = SegmentName::local("jeed.test.unlink.noop").unwrap();
    assert_eq!(SharedMapping::unlink(&name), Ok(()));
}

#[test]
fn an_impossible_size_fails_at_create_time_rather_than_later() {
    // 2^47 bytes: too big to commit against the pagefile, and too big for
    // `/dev/shm`. Which call notices differs; that it is noticed here does not.
    let name = unique("huge");
    assert!(matches!(SharedMapping::create(&name, 1 << 47), Err(ShmError::Os { .. })));
}
