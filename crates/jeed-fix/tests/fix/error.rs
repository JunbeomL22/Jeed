//! Tests for `src/data/fix/error.rs` — the error type must stay `Copy` and
//! allocation-free, and must name the tag or value at fault.

use jeed_fix::FixError;
use std::error::Error;

#[test]
fn every_variant_is_copy_and_carries_no_heap_data() {
    fn assert_copy<T: Copy>() {}
    assert_copy::<FixError>();
    // Two words at most: a discriminant plus the largest payload (two u32s).
    assert!(
        std::mem::size_of::<FixError>() <= 16,
        "FixError grew to {} bytes",
        std::mem::size_of::<FixError>()
    );
}

#[test]
fn messages_identify_what_went_wrong() {
    let cases = [
        (FixError::Incomplete, "incomplete"),
        (FixError::BeginString, "8="),
        (FixError::BodyLength { declared: 7 }, "7"),
        (FixError::Trailer, "10="),
        (FixError::CheckSum { expected: 12, found: 34 }, "012"),
        (FixError::Field { offset: 9 }, "9"),
        (FixError::MissingTag { tag: 34 }, "34"),
        (FixError::BadValue { tag: 270 }, "270"),
        (FixError::Precision { tag: 270 }, "270"),
        (FixError::UnexpectedMsgType { found: b'8' }, "'8'"),
        (FixError::TooManyEntries { declared: 33, max: 32 }, "33"),
        (FixError::EntryCount { declared: 3, found: 1 }, "3"),
        (FixError::GroupTagOutsideEntry { tag: 270 }, "270"),
        (FixError::UnknownUpdateAction { found: b'9' }, "'9'"),
    ];
    for (err, needle) in cases {
        let text = err.to_string();
        assert!(text.contains(needle), "{text:?} does not mention {needle:?}");
        assert!(text.starts_with("FIX"), "{text:?} does not say which layer failed");
        assert!(err.source().is_none());
    }
}

#[test]
fn errors_compare_by_value() {
    assert_eq!(FixError::MissingTag { tag: 34 }, FixError::MissingTag { tag: 34 });
    assert_ne!(FixError::MissingTag { tag: 34 }, FixError::MissingTag { tag: 35 });
    assert_ne!(FixError::BadValue { tag: 270 }, FixError::Precision { tag: 270 });
}
