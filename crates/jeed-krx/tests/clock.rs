//! `jeed_krx::clock` — assembling an absolute timestamp from a dateless one.

use jeed_krx::clock::{KST_OFFSET_NS, absolute_ns};

const DAY: u64 = 24 * 3600 * 1_000_000_000;

/// KST wall clock to nanoseconds since midnight.
const fn kst(h: u64, m: u64, s: u64) -> u64 {
    (h * 3600 + m * 60 + s) * 1_000_000_000
}

/// The absolute nanosecond of a KST wall clock on the day starting at
/// `utc_midnight`.
const fn at(utc_midnight: u64, h: u64, m: u64, s: u64) -> u64 {
    utc_midnight + kst(h, m, s) - KST_OFFSET_NS as u64
}

/// 2026-07-31 00:00:00 UTC.
const D: u64 = 1_785_456_000_000_000_000;

#[test]
fn a_reading_taken_moments_ago_lands_on_today() {
    let recv = at(D, 9, 1, 0) + 200_000_000;
    let tod = kst(9, 1, 0) + 123_456_000;
    assert_eq!(absolute_ns(tod, recv), at(D, 9, 1, 0) + 123_456_000);
    assert!(recv - absolute_ns(tod, recv) < 100_000_000_000);
}

#[test]
fn midnight_is_a_non_event() {
    // 23:59:59.9 stamped, received at 00:00:00.1 the next day. The day is
    // chosen as the one that puts the reading nearest reception, so this lands
    // 200 ms earlier — not 23 hours and 59 minutes later.
    let recv = at(D + DAY, 0, 0, 0) + 100_000_000;
    let tod = kst(23, 59, 59) + 900_000_000;
    let got = absolute_ns(tod, recv);
    assert_eq!(got, at(D, 23, 59, 59) + 900_000_000);
    assert_eq!(recv - got, 200_000_000);
}

#[test]
fn the_night_session_needs_no_special_case() {
    // Derivatives trade 18:00–05:00. A 01:30 stamp received at 01:30 belongs to
    // the calendar day it was received on, and nothing has to know that the
    // trading date is the previous one.
    let recv = at(D + DAY, 1, 30, 0) + 50_000_000;
    let tod = kst(1, 30, 0);
    assert_eq!(absolute_ns(tod, recv), at(D + DAY, 1, 30, 0));
}

#[test]
fn a_reading_from_just_after_reception_still_lands_on_today() {
    // Clock skew can put the exchange stamp slightly ahead of our reception.
    let recv = at(D, 9, 1, 0);
    let tod = kst(9, 1, 0) + 5_000_000;
    assert_eq!(absolute_ns(tod, recv), at(D, 9, 1, 0) + 5_000_000);
}

#[test]
fn the_chosen_day_is_always_the_nearest_one() {
    // The rule in one line: whatever comes out is within twelve hours of when
    // we received it. That is the whole guarantee, and it is what makes the
    // wrap and the night session both free.
    let recv = at(D, 12, 0, 0);
    for h in 0..24 {
        for m in [0u64, 17, 59] {
            let got = absolute_ns(kst(h, m, 0), recv);
            let delta = got.abs_diff(recv);
            assert!(delta <= DAY / 2, "{h:02}:{m:02} landed {delta} ns away");
        }
    }
}

#[test]
fn a_reading_before_the_epoch_saturates_instead_of_wrapping() {
    // Not a real feed, but `UnixNano` is unsigned and a negative would wrap
    // into the far future.
    assert_eq!(absolute_ns(0, 0), 0);
}
