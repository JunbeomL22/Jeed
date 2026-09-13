//! `jeed_shm::name` — segment names are validated before the OS sees them.

use jeed_shm::{MAX_NAME_LEN, SegmentName, ShmError};

#[cfg(windows)]
#[test]
fn a_windows_name_carries_its_namespace() {
    assert_eq!(SegmentName::local("jeed.krx.hot").unwrap().to_string(), "Local\\jeed.krx.hot");
    assert_eq!(SegmentName::global("jeed.krx.hot").unwrap().to_string(), "Global\\jeed.krx.hot");
}

#[cfg(windows)]
#[test]
fn on_windows_the_namespace_is_part_of_the_identity() {
    // Same text, different kernel object. Comparing the bare name would say
    // they are the same segment.
    assert_ne!(
        SegmentName::local("jeed.krx.hot").unwrap(),
        SegmentName::global("jeed.krx.hot").unwrap()
    );
}

#[cfg(unix)]
#[test]
fn a_posix_name_is_an_absolute_shm_name() {
    assert_eq!(SegmentName::local("jeed.krx.hot").unwrap().to_string(), "/jeed.krx.hot");
}

#[cfg(unix)]
#[test]
fn on_posix_the_two_namespaces_are_one() {
    // POSIX shared memory has a single namespace scoped by file permissions, so
    // these name the same object — and equality has to say so, because the
    // question it answers is "is this the same segment?".
    assert_eq!(
        SegmentName::local("jeed.krx.hot").unwrap(),
        SegmentName::global("jeed.krx.hot").unwrap()
    );
}

#[test]
fn an_empty_name_is_refused() {
    assert_eq!(SegmentName::local(""), Err(ShmError::NameEmpty));
}

#[test]
fn a_name_at_the_limit_is_accepted_and_one_past_is_not() {
    let at = "a".repeat(MAX_NAME_LEN);
    assert!(SegmentName::local(&at).is_ok());

    let over = "a".repeat(MAX_NAME_LEN + 1);
    assert_eq!(SegmentName::local(&over), Err(ShmError::NameTooLong { len: MAX_NAME_LEN + 1 }));
}

#[test]
fn a_separator_cannot_be_smuggled_in_from_config() {
    // A backslash would reach into another Windows namespace and a slash would
    // make a POSIX name the kernel refuses outright. Neither is something a
    // config string should be able to do.
    assert_eq!(SegmentName::local("Global\\jeed"), Err(ShmError::NameChar { byte: b'\\' }));
    assert_eq!(SegmentName::local("jeed/krx"), Err(ShmError::NameChar { byte: b'/' }));
}

#[test]
fn non_printable_and_non_ascii_are_refused() {
    assert_eq!(SegmentName::local("jeed krx"), Err(ShmError::NameChar { byte: b' ' }));
    assert_eq!(SegmentName::local("jeed\0krx"), Err(ShmError::NameChar { byte: 0 }));
    assert!(matches!(SegmentName::local("시세"), Err(ShmError::NameChar { .. })));
}

#[test]
fn names_are_cloneable_without_reallocation_at_use_time() {
    let n = SegmentName::local("jeed.krx.hot").unwrap();
    let c = n.clone();
    assert_eq!(n, c);
    assert_ne!(n.as_ptr(), c.as_ptr(), "the buffer is inline, so a clone owns its own");
}
