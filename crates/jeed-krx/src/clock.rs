//! Turning `HHMMSSuuuuuu` into an absolute timestamp.
//!
//! KRX stamps 매매처리시각 as a wall clock with **no date**, so it wraps at
//! midnight — and the derivatives night session runs straight through one.
//! Assembling the absolute nanosecond is the handler's job and ends here; the
//! wire carries only the assembled value (`documents/feed_handler.md` §6).

use crate::field::NS_PER_DAY;
use jeed_wire::UnixNano;

/// KRX publishes in KST, which has no daylight saving and has been UTC+9 for
/// the whole life of the exchange's electronic feeds.
pub const KST_OFFSET_NS: i64 = 9 * 3600 * 1_000_000_000;

/// Absolute nanoseconds for a KST time-of-day, using `recv_ns` to pick the day.
///
/// The day is chosen as the one that puts the reading **nearest** the moment we
/// received it. That is what makes the midnight wrap a non-event: a 23:59:59
/// stamp received at 00:00:01 lands on the previous day without anyone having
/// to know the trading date, and a night-session stamp does the same. It only
/// misattributes if the exchange clock and ours disagree by twelve hours, which
/// is a different kind of problem.
///
/// No calendar and no configuration: a trading-date setting is one more thing
/// to get wrong at 18:00 on a rollover evening.
#[inline]
pub const fn absolute_ns(time_of_day_ns: u64, recv_ns: UnixNano) -> UnixNano {
    let day = NS_PER_DAY as i64;
    // The reading as an offset from the UTC epoch, before choosing a day.
    let base = time_of_day_ns as i64 - KST_OFFSET_NS;
    let recv = recv_ns as i64;

    // Largest `k` with `base + k*day <= recv`.
    let k = (recv - base).div_euclid(day);
    let before = base + k * day;
    let after = before + day;

    let to_before = recv - before;
    let to_after = after - recv;
    let chosen = if to_before <= to_after { before } else { after };

    if chosen < 0 { 0 } else { chosen as UnixNano }
}
