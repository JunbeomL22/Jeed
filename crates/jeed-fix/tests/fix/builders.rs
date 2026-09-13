//! Message builders shared by every test target in this crate.
//!
//! A third, deliberately naive encoder. The library must not be used to produce
//! the bytes it is being tested on, and `jeed_fix::recv::emit` — the real
//! encoder — must not be either: `recv/emit.rs` is checked *against* these, so
//! a bug shared by both would otherwise cancel out.
//!
//! `tests/recv` reaches this file by `#[path]` rather than copying it, the same
//! way `jeed_krx::tests::recv` reaches the decoder tests' 전문 builders.

use jeed_fix::{FixScales, SOH};
use jeed_wire::Scale;

/// Scales of the SMBS USDKRW capture: two-decimal prices, whole-USD sizes.
pub const SCALES: FixScales = FixScales::new(Scale::S2, Scale::S0);

/// Turns `|`-delimited text into SOH-delimited bytes.
pub fn soh(text: &str) -> Vec<u8> {
    text.bytes().map(|b| if b == b'|' { SOH } else { b }).collect()
}

/// Frames a `|`-delimited body (everything between `9=` and `10=`) into a
/// complete `FIX.4.4` message with a correct `BodyLength` and `CheckSum`.
pub fn wrap(body: &str) -> Vec<u8> {
    wrap_as("FIX.4.4", body)
}

/// [`wrap`] with an explicit `BeginString`.
pub fn wrap_as(begin_string: &str, body: &str) -> Vec<u8> {
    let mut body = soh(body);
    if body.last() != Some(&SOH) {
        body.push(SOH);
    }
    let mut out = Vec::with_capacity(body.len() + 32);
    out.extend_from_slice(format!("8={begin_string}").as_bytes());
    out.push(SOH);
    out.extend_from_slice(format!("9={}", body.len()).as_bytes());
    out.push(SOH);
    out.extend_from_slice(&body);
    let sum: u32 = out.iter().map(|b| u32::from(*b)).sum();
    out.extend_from_slice(format!("10={:03}", sum % 256).as_bytes());
    out.push(SOH);
    out
}

/// The first message of `E:/Data/smbs_fix_db/20260202` — a one-sided book at
/// the open, which is the shape that breaks a careless decoder.
pub const OPEN_SNAPSHOT: &str = "35=W|49=SMBS|56=FRACTAL|34=1|52=20260202-00:00:00.005|\
     262=USDKRW-SMBS|55=USDKRW|268=1|269=1|270=1453.00|271=5000000|\
     272=20260202|273=00:00:00.000";

/// A two-sided snapshot: bid 1450.00, offer 1453.00, 5 mio each.
pub const TWO_SIDED_SNAPSHOT: &str = "35=W|49=SMBS|56=FRACTAL|34=2|52=20260202-00:00:01.005|\
     262=USDKRW-SMBS|55=USDKRW|268=2|\
     269=0|270=1450.00|271=5000000|272=20260202|273=00:00:01.000|\
     269=1|270=1453.00|271=5000000|272=20260202|273=00:00:01.000";

/// An incremental refresh carrying one print of 1 mio at 1451.00.
pub const TRADE_INCREMENTAL: &str = "35=X|49=SMBS|56=FRACTAL|34=18|52=20260202-00:00:14.005|\
     262=USDKRW-SMBS|268=1|279=0|269=2|55=USDKRW|270=1451.00|271=1000000|\
     272=20260202|273=00:00:14.000";

/// A session heartbeat.
pub const HEARTBEAT: &str = "35=0|49=SMBS|56=FRACTAL|34=70|52=20260202-00:00:30.005";

/// Venue midnight of 2026-02-02 UTC in unix ns (from the capture index).
pub const DAY_NS: u64 = 1_769_990_400_000_000_000;
