//! `@depth` — Binance USD-M incremental updates.

use crate::common::*;
use jeed_wire::{WireDeltaLevel, WireKind, WireRecord, delta_flags};
use jeed_crypto::binance::futures::delta;

#[test]
fn decodes_the_documented_frame() {
    let inst = futures_btcusdt();
    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, FUT_DELTA, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.kind(), Ok(WireKind::SnapshotDelta));
    assert_eq!(rec.validate(), Ok(()));

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.first_update_id, 390_497_796);
    assert_eq!(d.final_update_id, 390_497_878);
    assert_eq!(
        d.bids(),
        &[
            WireDeltaLevel { price: 740_389, qty: 2 },
            WireDeltaLevel { price: 740_388, qty: 0 },
        ]
    );
    assert_eq!(d.asks(), &[WireDeltaLevel { price: 740_596, qty: 3_340 }]);
}

#[test]
fn pu_names_the_message_this_one_must_follow() {
    // USD-M does not guarantee `U == last u + 1` across a reconnect, so `pu`
    // is the chain the consumer can actually check.
    let inst = futures_btcusdt();
    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, FUT_DELTA, RECV_NS, &mut rec).unwrap();

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.prev_final_update_id(), Some(390_497_794));
    assert!(d.delta_flags & delta_flags::PREV_FINAL_VALID != 0);
}

#[test]
fn a_pu_of_zero_is_a_real_pu() {
    // The first message after a stream starts. Without the flag this would be
    // indistinguishable from spot, which sends no `pu` at all.
    let inst = futures_btcusdt();
    let frame = br#"{"e":"depthUpdate","E":1,"T":1,"s":"BTCUSDT","U":1,"u":2,"pu":0,"b":[],"a":[]}"#;
    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    assert_eq!(rec.snapshot_delta().unwrap().prev_final_update_id(), Some(0));
}

#[test]
fn a_frame_without_pu_still_decodes() {
    // Not required: `pu` is USD-M's convenience and `U`/`u` still chain.
    let inst = futures_btcusdt();
    let frame = br#"{"e":"depthUpdate","E":1,"T":1,"s":"BTCUSDT","U":1,"u":2,"b":[],"a":[]}"#;
    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, frame, RECV_NS, &mut rec).unwrap();

    let d = rec.snapshot_delta().unwrap();
    assert_eq!(d.prev_final_update_id(), None);
    assert_eq!(d.final_update_id, 2);
}

#[test]
fn venue_time_prefers_t() {
    let inst = futures_btcusdt();
    let mut rec = WireRecord::zeroed();
    delta::decode(&inst, FUT_DELTA, RECV_NS, &mut rec).unwrap();
    assert_eq!(rec.header.venue_time(), Some(1_571_889_248_276_000_000));
}
