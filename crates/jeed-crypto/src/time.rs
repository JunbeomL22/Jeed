//! Turning a venue's idea of a timestamp into `venue_ns`.
//!
//! Crypto venues agree on nothing here. Most send milliseconds since the
//! epoch, KuCoin sends nanoseconds, and Gate sends milliseconds with a
//! fractional part. All of it has to become the one thing
//! the wire carries — nanoseconds since the epoch, in
//! [`RecordHeader::venue_ns`](jeed_wire::RecordHeader::venue_ns) — before it
//! reaches a record, because a consumer that had to know which venue sent a
//! record to read its clock would be reading evidence rather than a conclusion
//! (`documents/feed_handler.md` §8).
//!
//! Every reader here saturates rather than wrapping. A garbage timestamp must
//! not come out the far side as a plausible time, and `u64::MAX` nanoseconds
//! is the year 2554, so saturation is only ever reached by nonsense.

use jeed_wire::UnixNano;

/// Milliseconds since the epoch, as most venues timestamp, in nanoseconds.
#[inline]
pub const fn millis_to_nanos(ms: u64) -> UnixNano {
    ms.saturating_mul(1_000_000)
}

/// Milliseconds with a fractional part — Gate's `"1606292218213.4578"` — in
/// nanoseconds.
///
/// The fraction is kept rather than dropped: those are real sub-millisecond
/// digits the venue measured, and a millisecond is a long time on a trade
/// tape. Digits past the sixth are dropped, which is the timestamp being finer
/// than a nanosecond.
///
/// `None` when the text holds no leading digit, or anything that is not a
/// digit after the point — a malformed clock reading must not become a
/// plausible time.
///
/// # Example
/// ```
/// use jeed_crypto::time::decimal_millis_to_nanos;
///
/// assert_eq!(decimal_millis_to_nanos(b"1606292218213.4578"), Some(1_606_292_218_213_457_800));
/// assert_eq!(decimal_millis_to_nanos(b"1606292218213"), Some(1_606_292_218_213_000_000));
/// assert_eq!(decimal_millis_to_nanos(b"later"), None);
/// ```
pub fn decimal_millis_to_nanos(bytes: &[u8]) -> Option<UnixNano> {
    let mut millis: u64 = 0;
    let mut digits = 0usize;
    let mut i = 0usize;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        millis = millis.saturating_mul(10).saturating_add((bytes[i] - b'0') as u64);
        digits += 1;
        i += 1;
    }
    if digits == 0 {
        return None;
    }

    let nanos = millis_to_nanos(millis);
    if i == bytes.len() {
        return Some(nanos);
    }
    if bytes[i] != b'.' {
        return None;
    }

    // A millisecond is a million nanoseconds, so six fractional digits fit.
    let mut frac: u64 = 0;
    let mut kept = 0usize;
    i += 1;
    let mut seen = 0usize;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            return None;
        }
        if kept < 6 {
            frac = frac * 10 + (bytes[i] - b'0') as u64;
            kept += 1;
        }
        seen += 1;
        i += 1;
    }
    if seen == 0 {
        return None;
    }
    while kept < 6 {
        frac *= 10;
        kept += 1;
    }

    Some(nanos.saturating_add(frac))
}
