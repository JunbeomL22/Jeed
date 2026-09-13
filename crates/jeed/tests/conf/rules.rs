//! `jeed::conf::rules` — the placement rules both confs share, on their own.

use jeed::conf::RuleError;
use jeed::conf::rules::{Placement, check_placement};
use jeed::cpu::Topology;

fn feed<'a>(name: &'a str, spin: bool, cores: &'a [u16], ring: &'a str) -> Placement<'a> {
    Placement { name, spin, cores, ring, ring_slots: 64 }
}

/// Four logical processors: (0,1) share a core, (2,3) share a core.
fn two_cores_smt() -> Topology {
    Topology::from_siblings(vec![0b0011, 0b0011, 0b1100, 0b1100])
}

#[test]
fn a_clean_layout_passes_with_and_without_topology() {
    let feeds = [feed("hot", true, &[2], "jeed.test.hot"), feed("cold", false, &[0, 1], "jeed.test.cold")];
    check_placement(&feeds, None).unwrap();
    check_placement(&feeds, Some(&two_cores_smt())).unwrap();
}

#[test]
fn nothing_and_nameless_are_refused_first() {
    assert_eq!(check_placement(&[], None), Err(RuleError::NoFeeds));
    assert_eq!(check_placement(&[feed("", true, &[0], "jeed.test.x")], None), Err(RuleError::EmptyName { index: 0 }));
    let feeds = [feed("a", false, &[0], "jeed.test.a"), feed("a", false, &[1], "jeed.test.b")];
    assert_eq!(check_placement(&feeds, None), Err(RuleError::DuplicateName { name: "a".into() }));
}

#[test]
fn rings_are_one_producer_each_and_a_power_of_two() {
    let feeds = [feed("a", false, &[0], "jeed.test.r"), feed("b", false, &[1], "jeed.test.r")];
    assert_eq!(
        check_placement(&feeds, None),
        Err(RuleError::DuplicateRing { ring: "jeed.test.r".into(), a: "a".into(), b: "b".into() })
    );
    let odd = Placement { ring_slots: 48, ..feed("a", false, &[0], "jeed.test.a") };
    assert_eq!(check_placement(&[odd], None), Err(RuleError::RingSlots { feed: "a".into(), slots: 48 }));
    let bad = feed("a", false, &[0], "");
    assert!(matches!(check_placement(&[bad], None), Err(RuleError::Ring { feed, .. }) if feed == "a"));
}

#[test]
fn cores_are_exclusive_and_a_spinner_has_exactly_one() {
    assert_eq!(check_placement(&[feed("a", false, &[], "jeed.test.a")], None), Err(RuleError::NoCores { feed: "a".into() }));
    assert_eq!(
        check_placement(&[feed("a", true, &[0, 1], "jeed.test.a")], None),
        Err(RuleError::SpinOnSeveralCores { feed: "a".into(), cores: 2 })
    );
    let feeds = [feed("a", false, &[0, 1], "jeed.test.a"), feed("b", false, &[1], "jeed.test.b")];
    assert_eq!(check_placement(&feeds, None), Err(RuleError::CoreShared { core: 1, a: "a".into(), b: "b".into() }));
    assert_eq!(
        check_placement(&[feed("a", false, &[4], "jeed.test.a")], Some(&two_cores_smt())),
        Err(RuleError::CoreOutOfRange { feed: "a".into(), core: 4, logical: 4 })
    );
    assert_eq!(
        check_placement(&[feed("a", false, &[64], "jeed.test.a")], None),
        Err(RuleError::CoreOutOfRange { feed: "a".into(), core: 64, logical: 64 })
    );
}

#[test]
fn a_spinners_smt_sibling_is_off_limits_only_when_the_topology_is_known() {
    let feeds = [feed("hot", true, &[2], "jeed.test.hot"), feed("cold", false, &[3], "jeed.test.cold")];
    check_placement(&feeds, None).unwrap();
    assert_eq!(
        check_placement(&feeds, Some(&two_cores_smt())),
        Err(RuleError::SiblingShared { spinning: "hot".into(), core: 2, sibling: 3, other: "cold".into() })
    );
    // Two blocking feeds may share a physical core.
    let feeds = [feed("a", false, &[2], "jeed.test.a"), feed("b", false, &[3], "jeed.test.b")];
    check_placement(&feeds, Some(&two_cores_smt())).unwrap();
}

#[test]
fn messages_read_as_sentences() {
    assert_eq!(RuleError::NoInstruments { feed: "x".into() }.to_string(), "feed[x]: no [[feed.instrument]]");
    assert_eq!(
        RuleError::SiblingShared { spinning: "hot".into(), core: 2, sibling: 3, other: "cold".into() }.to_string(),
        "feed \"cold\" uses core 3, the SMT sibling of core 2 that \"hot\" spins on"
    );
}
