//! `jeed::conf::fix` — `conf/fix.toml`.

use jeed::conf::rules::RuleError;
use jeed::conf::{ConfError, FixConf};
use jeed::toml::parse;
use jeed_fix::recv::{Mode, SubscriptionType};
use jeed_wire::{Scale, Venue};

const MINIMAL: &str = r#"
[[feed]]
name = "smbs"
venue = "smbs"
mode = "block"
cores = [4]
ring = "jeed.test.fix"
ring_slots = 1024
host = "10.0.0.1"
port = 9100
sender_comp_id = "JEED"
target_comp_id = "SMBS"
price_decimals = 2
qty_decimals = 0
symbols = ["USD/KRW", "EUR/KRW"]
"#;

fn conf(text: &str) -> Result<FixConf, ConfError> {
    FixConf::from_table(&parse(text).unwrap())
}

#[test]
fn a_minimal_conf_gets_the_session_defaults() {
    let c = conf(MINIMAL).unwrap();
    assert_eq!(c.feeds.len(), 1);
    let f = &c.feeds[0];
    assert_eq!(f.venue, Venue::Smbs);
    assert_eq!(f.mode, Mode::Block);
    assert_eq!(f.begin_string, "FIX.4.4");
    assert_eq!(f.heartbeat_secs, 30);
    assert!(f.reset_seq);
    assert_eq!(f.reconnect_secs, 5);
    assert_eq!(f.logon_timeout_secs, 10);
    assert_eq!(f.burst, 64);
    assert_eq!(f.depth, 0, "full book");
    assert_eq!(f.subscription, SubscriptionType::SnapshotPlusUpdates);
    assert_eq!(f.default_qty, None, "a level with no size is refused, not invented");
    assert_eq!(f.symbols, ["USD/KRW", "EUR/KRW"]);
    assert_eq!(f.scales().price, Scale::S2);
    assert_eq!(f.scales().quantity, Scale::S0);
    assert_eq!(f.endpoint().to_string(), "10.0.0.1:9100");
    assert_eq!(c.report_secs, 10);
    assert_eq!(c.health.stale_ns, 500_000_000);

    let cfg = f.receiver_config(c.health);
    assert_eq!(cfg.heartbeat_secs, 30);
    assert_eq!(cfg.reconnect_ns, 5_000_000_000);
    assert_eq!(cfg.logon_timeout_ns, 10_000_000_000);
    assert_eq!(cfg.stale_ns, 500_000_000);
}

#[test]
fn every_session_knob_is_read() {
    let text = format!(
        "{MINIMAL}\nbegin_string = \"FIX.4.2\"\nheartbeat_secs = 20\nreset_seq = false\n\
         reconnect_secs = 2\nlogon_timeout_secs = 3\nburst = 8\ndepth = 1\nsubscription = \"snapshot\"\n\
         default_qty = 5000000\n[health]\nstale_ms = 0\n[log]\nreport_secs = 0"
    );
    let c = conf(&text).unwrap();
    let f = &c.feeds[0];
    assert_eq!(f.begin_string, "FIX.4.2");
    assert_eq!(f.heartbeat_secs, 20);
    assert!(!f.reset_seq);
    assert_eq!(f.reconnect_secs, 2);
    assert_eq!(f.logon_timeout_secs, 3);
    assert_eq!(f.burst, 8);
    assert_eq!(f.depth, 1);
    assert_eq!(f.subscription, SubscriptionType::Snapshot);
    assert_eq!(f.default_qty, Some(5_000_000));
    assert_eq!(c.health.stale_ns, 0);
    assert_eq!(c.report_secs, 0);
}

#[test]
fn the_example_conf_loads_and_validates() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conf/fix.example.toml");
    let c = FixConf::load(&path).unwrap();
    c.validate(None).unwrap();
    assert_eq!(c.feeds[0].name, "smbs");
}

#[test]
fn an_unknown_key_is_refused_by_name() {
    let e = conf(&format!("{MINIMAL}\nsymbol = [\"USD/KRW\"]")).unwrap_err();
    assert!(matches!(&e, ConfError::Unknown { at, key } if at == "feed[smbs]" && key == "symbol"), "{e}");
}

#[test]
fn an_unknown_venue_names_the_known_ones() {
    let e = conf(&MINIMAL.replace("venue = \"smbs\"", "venue = \"cme\"")).unwrap_err();
    assert!(e.to_string().contains("not a FIX venue"), "{e}");
    assert!(e.to_string().contains("smbs"), "{e}");
}

#[test]
fn bad_values_are_refused_with_the_reason() {
    for (from, to, expect) in [
        ("port = 9100", "port = 0", "1..=65535"),
        ("port = 9100", "port = 70000", "1..=65535"),
        ("price_decimals = 2", "price_decimals = 9", "0..=8"),
        ("mode = \"block\"", "mode = \"poll\"", "spin"),
    ] {
        let e = conf(&MINIMAL.replace(from, to)).unwrap_err();
        assert!(e.to_string().contains(expect), "{to}: {e}");
    }
    for extra in ["reconnect_secs = 0", "logon_timeout_secs = 0", "burst = 0", "subscription = \"all\""] {
        assert!(conf(&format!("{MINIMAL}\n{extra}")).is_err(), "{extra}");
    }
}

#[test]
fn a_symbol_that_does_not_fit_the_wire_is_refused_at_parse() {
    let long = "X".repeat(jeed_wire::SYMBOL_LEN + 1);
    let e = conf(&MINIMAL.replace("\"EUR/KRW\"", &format!("\"{long}\""))).unwrap_err();
    assert!(e.to_string().contains("longer than the wire symbol field"), "{e}");
    let e = conf(&MINIMAL.replace("\"EUR/KRW\"", "\"\"")).unwrap_err();
    assert!(e.to_string().contains("empty"), "{e}");
}

#[test]
fn validate_applies_the_shared_placement_rules_and_wants_symbols() {
    let c = conf(MINIMAL).unwrap();
    c.validate(None).unwrap();

    let e = conf(&MINIMAL.replace("ring_slots = 1024", "ring_slots = 1000")).unwrap().validate(None).unwrap_err();
    assert!(matches!(e, RuleError::RingSlots { .. }), "{e}");

    let e = conf(&MINIMAL.replace("symbols = [\"USD/KRW\", \"EUR/KRW\"]", "symbols = []"))
        .unwrap()
        .validate(None)
        .unwrap_err();
    assert!(matches!(e, RuleError::NoInstruments { .. }), "{e}");
}

#[test]
fn a_symbol_under_two_feeds_is_a_warning_not_a_refusal() {
    let second = MINIMAL.replace("name = \"smbs\"", "name = \"smbs-b\"").replace("cores = [4]", "cores = [6]").replace("jeed.test.fix", "jeed.test.fix2");
    let c = conf(&format!("{MINIMAL}\n{second}")).unwrap();
    c.validate(None).unwrap();
    let w = c.warnings(None);
    assert!(w.iter().any(|w| w.to_string().contains("USD/KRW") && w.to_string().contains("two rings")), "{w:?}");
}
