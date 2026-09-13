//! `jeed_crypto::time` — venue clocks, in the one unit the wire carries.

use jeed_crypto::time::{decimal_millis_to_nanos, millis_to_nanos};

#[test]
fn milliseconds_become_nanoseconds() {
    assert_eq!(millis_to_nanos(0), 0);
    assert_eq!(millis_to_nanos(1_706_000_000_000), 1_706_000_000_000_000_000);
}

#[test]
fn a_nonsense_millisecond_saturates_rather_than_wrapping() {
    // Wrapping would turn a garbage field into a plausible recent time, which
    // is the one outcome worse than an obviously wrong one.
    assert_eq!(millis_to_nanos(u64::MAX), u64::MAX);
}

#[test]
fn a_fractional_millisecond_keeps_its_digits() {
    assert_eq!(decimal_millis_to_nanos(b"1606292218213.4578"), Some(1_606_292_218_213_457_800));
    assert_eq!(decimal_millis_to_nanos(b"1606292218213.5"), Some(1_606_292_218_213_500_000));
    assert_eq!(decimal_millis_to_nanos(b"0.000001"), Some(1), "one nanosecond");
}

#[test]
fn an_integer_reads_the_same_either_way() {
    // Gate quotes `create_time_ms` with a fraction most of the time and
    // without one sometimes; both are the same instant.
    assert_eq!(decimal_millis_to_nanos(b"1606292218213"), Some(1_606_292_218_213_000_000));
    assert_eq!(decimal_millis_to_nanos(b"1606292218213"), Some(millis_to_nanos(1_606_292_218_213)));
}

#[test]
fn digits_finer_than_a_nanosecond_are_dropped_and_not_refused() {
    // Seven fractional digits of a millisecond is sub-nanosecond precision.
    // That is the venue being precise, not the field being broken.
    assert_eq!(decimal_millis_to_nanos(b"1606292218213.4578912"), Some(1_606_292_218_213_457_891));
}

#[test]
fn text_that_is_not_a_number_is_no_time_at_all() {
    assert_eq!(decimal_millis_to_nanos(b""), None);
    assert_eq!(decimal_millis_to_nanos(b"later"), None);
    assert_eq!(decimal_millis_to_nanos(b".5"), None, "no whole part");
    assert_eq!(decimal_millis_to_nanos(b"1606292218213."), None, "a point with nothing after it");
    assert_eq!(decimal_millis_to_nanos(b"1606292218213.45x"), None);
}
