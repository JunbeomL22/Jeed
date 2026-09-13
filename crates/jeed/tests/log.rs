//! `jeed::log` — the timestamp, which is the only part with arithmetic in it.

use jeed::log::{Timestamp, civil_from_days};

#[test]
fn the_epoch() {
    assert_eq!(Timestamp(0).to_string(), "1970-01-01T00:00:00.000Z");
}

#[test]
fn a_known_instant() {
    // 2023-11-14 22:13:20 UTC, plus 123 ms.
    assert_eq!(Timestamp(1_700_000_000_123_456_789).to_string(), "2023-11-14T22:13:20.123Z");
}

#[test]
fn civil_dates_across_leap_years_and_centuries() {
    assert_eq!(civil_from_days(0), (1970, 1, 1));
    assert_eq!(civil_from_days(-1), (1969, 12, 31));
    assert_eq!(civil_from_days(11_016), (2000, 2, 29), "a leap day in a leap century");
    assert_eq!(civil_from_days(11_017), (2000, 3, 1));
    assert_eq!(civil_from_days(10_957), (2000, 1, 1));
    assert_eq!(civil_from_days(20_709), (2026, 9, 13));
    assert_eq!(civil_from_days(19_723), (2024, 1, 1));
}

#[test]
fn the_day_boundary() {
    let last_ns_of_day = 86_399_999_999_999;
    assert_eq!(Timestamp(last_ns_of_day).to_string(), "1970-01-01T23:59:59.999Z");
    assert_eq!(Timestamp(last_ns_of_day + 1).to_string(), "1970-01-02T00:00:00.000Z");
}
