//! `jeed::conf` — one test target whose module tree mirrors `src/conf`.

use jeed::conf::{ConfError, Health, Section};
use jeed::toml::parse;

mod krx;
mod trcodes;

#[test]
fn health_defaults_have_both_guards_on() {
    // A conf without `[health]` gets §9's values, not zero: zero once froze a
    // book for twenty minutes.
    let h = Health::from_section(None).unwrap();
    assert_eq!(h.heartbeat_ns, 100_000_000);
    assert_eq!(h.stale_ns, 500_000_000);
}

#[test]
fn health_reads_milliseconds_and_accepts_an_explicit_zero() {
    let t = parse("[health]\nheartbeat_ms = 250\nstale_ms = 0").unwrap();
    let s = Section::new("", &t).optional_table("health").unwrap();
    let h = Health::from_section(s).unwrap();
    assert_eq!(h.heartbeat_ns, 250_000_000);
    assert_eq!(h.stale_ns, 0, "off, because the conf said so");
}

#[test]
fn a_section_refuses_keys_it_does_not_know() {
    let t = parse("[health]\nstale_secs = 1").unwrap();
    let s = Section::new("", &t).optional_table("health").unwrap();
    let e = Health::from_section(s).unwrap_err();
    assert!(matches!(&e, ConfError::Unknown { at, key } if at == "health" && key == "stale_secs"), "{e}");
    assert_eq!(e.to_string(), "health: unknown key `stale_secs`");
}

#[test]
fn a_section_names_where_and_what_went_wrong() {
    let t = parse("name = 5\ncores = [1, \"two\"]\nn = -1").unwrap();
    let s = Section::new("feed[hot]", &t);

    let e = s.required_str("name").unwrap_err();
    assert_eq!(e.to_string(), "feed[hot]: `name` must be a string, found integer");

    let e = s.required_str("ring").unwrap_err();
    assert_eq!(e.to_string(), "feed[hot]: `ring` is required");

    let e = s.required_u16s("cores").unwrap_err();
    assert_eq!(e.to_string(), "feed[hot]: `cores` must be an array of integers, found string");

    let e = s.optional_u64("n").unwrap_err();
    assert_eq!(e.to_string(), "feed[hot]: `n` must not be negative");

    let root = Section::new("", &t);
    assert_eq!(root.required_str("x").unwrap_err().to_string(), "top level: `x` is required");
}
