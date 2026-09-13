//! `jeed::conf::krx` — reading and validating `conf/krx.toml`.

use jeed::conf::krx::{RuleError, Warning};
use jeed::conf::{ConfError, KrxConf, TrCodeTable};
use jeed::cpu::Topology;
use jeed::toml::parse;
use jeed_krx::TrCode;
use jeed_krx::recv::Mode;

const MINIMAL: &str = r#"
[[feed]]
name = "hot"
mode = "spin"
cores = [2]
ring = "jeed.test.hot"
ring_slots = 4096
sockets = ["239.255.77.92:30302"]
trcodes = ["B601F", "G701F"]

[[feed]]
name = "cold"
mode = "block"
cores = [4, 5]
ring = "jeed.test.cold"
ring_slots = 256
sockets = ["239.255.77.93:30315"]
trcodes = ["M401F"]
"#;

fn conf(text: &str) -> KrxConf {
    KrxConf::from_table(&parse(text).unwrap()).unwrap()
}

fn code(s: &str) -> TrCode {
    TrCode::from_message(s.as_bytes()).unwrap()
}

/// Four logical processors: (0,1) share a core, (2,3) share a core.
fn two_cores_smt() -> Topology {
    Topology::from_siblings(vec![0b0011, 0b0011, 0b1100, 0b1100])
}

/// Eight, no SMT.
fn eight_flat() -> Topology {
    Topology::from_siblings((0..8).map(|i| 1 << i).collect())
}

// ── reading ─────────────────────────────────────────────────────────────

#[test]
fn the_minimal_conf_reads_with_defaults() {
    let c = conf(MINIMAL);
    assert_eq!(c.feeds.len(), 2);
    assert_eq!(c.trcode_table, None);
    assert_eq!(c.isin_list, None);
    assert_eq!(c.report_secs, 10);
    assert_eq!(c.health.stale_ns, 500_000_000);

    let hot = &c.feeds[0];
    assert_eq!(hot.name, "hot");
    assert_eq!(hot.mode, Mode::Spin);
    assert_eq!(hot.cores, [2]);
    assert_eq!(hot.ring, "jeed.test.hot");
    assert_eq!(hot.ring_slots, 4096);
    assert_eq!(hot.sockets.len(), 1);
    assert_eq!(hot.sockets[0].port, 30302);
    assert_eq!(hot.trcodes, [code("B601F"), code("G701F")]);
    assert_eq!(hot.burst, 64);
    assert_eq!(hot.recv_buffer_bytes, 8 << 20);

    let cfg = hot.receiver_config(c.health);
    assert_eq!(cfg.mode, Mode::Spin);
    assert_eq!(cfg.heartbeat_ns, 100_000_000);
    assert_eq!(cfg.stale_ns, 500_000_000);
    assert_eq!(cfg.burst, 64);
    assert_eq!(cfg.socket.recv_buffer_bytes, 8 << 20);
    assert!(cfg.socket.nonblocking);

    assert_eq!(c.feeds[1].mode, Mode::Block);
    assert_eq!(c.feeds[1].mask(), Some(0b11_0000));
}

#[test]
fn every_optional_key_is_read() {
    let text = format!(
        "trcode_table = \"conf/krx_trcodes.toml\"\n{MINIMAL}\n\
         [filter]\nisin = \"conf/isin_allow.txt\"\n\
         [health]\nheartbeat_ms = 50\nstale_ms = 0\n\
         [log]\nreport_secs = 0\n"
    );
    let text = text.replace("ring_slots = 4096", "ring_slots = 4096\nburst = 16\nrecv_buffer_bytes = 1048576");
    let c = conf(&text);
    assert_eq!(c.trcode_table.as_deref().unwrap().to_str(), Some("conf/krx_trcodes.toml"));
    assert_eq!(c.isin_list.as_deref().unwrap().to_str(), Some("conf/isin_allow.txt"));
    assert_eq!(c.health.heartbeat_ns, 50_000_000);
    assert_eq!(c.health.stale_ns, 0);
    assert_eq!(c.report_secs, 0);
    assert_eq!(c.feeds[0].burst, 16);
    assert_eq!(c.feeds[0].recv_buffer_bytes, 1 << 20);
}

fn read_err(text: &str) -> String {
    KrxConf::from_table(&parse(text).unwrap()).unwrap_err().to_string()
}

#[test]
fn a_typo_is_an_error_not_a_default() {
    // `ring_slot` next to a defaulted `ring_slots` would run with the wrong
    // size and say nothing.
    let text = MINIMAL.replace("ring_slots = 4096", "ring_slots = 4096\nring_slot = 8");
    assert_eq!(read_err(&text), "feed[hot]: unknown key `ring_slot`");
    assert_eq!(read_err(&format!("{MINIMAL}\n[helth]\nstale_ms = 1")), "top level: unknown key `helth`");
    assert_eq!(read_err(&format!("{MINIMAL}\n[log]\nreport_sec = 1")), "log: unknown key `report_sec`");
}

#[test]
fn the_feed_is_named_in_its_errors_once_it_has_a_name() {
    let text = MINIMAL.replace("mode = \"spin\"", "mode = \"fast\"");
    assert_eq!(read_err(&text), "feed[hot]: `mode` must be \"spin\" or \"block\", found \"fast\"");

    let text = MINIMAL.replacen("name = \"hot\"\n", "", 1);
    assert_eq!(read_err(&text), "feed[0]: `name` is required");
}

#[test]
fn sockets_and_trcodes_are_checked_as_they_are_read() {
    let text = MINIMAL.replace("239.255.77.92:30302", "10.0.0.1:30302");
    let e = read_err(&text);
    assert!(e.starts_with("feed[hot]: `sockets` `10.0.0.1:30302`:"), "{e}");

    let text = MINIMAL.replace("\"G701F\"", "\"G701\"");
    assert_eq!(read_err(&text), "feed[hot]: `trcodes` `G701` is not a five-byte trcode");

    let text = MINIMAL.replace("ring_slots = 4096", "ring_slots = 4096\nburst = 0");
    assert_eq!(read_err(&text), "feed[hot]: `burst` must be at least 1");
}

// ── rules ───────────────────────────────────────────────────────────────

fn rule(text: &str, topology: Option<&Topology>) -> RuleError {
    conf(text).validate(topology).unwrap_err()
}

#[test]
fn the_minimal_conf_is_valid_with_or_without_a_topology() {
    let c = conf(MINIMAL);
    c.validate(None).unwrap();
    c.validate(Some(&eight_flat())).unwrap();
}

#[test]
fn no_feeds_is_refused() {
    assert_eq!(conf("[health]\nstale_ms = 1").validate(None).unwrap_err(), RuleError::NoFeeds);
}

#[test]
fn names_and_rings_are_unique() {
    let text = MINIMAL.replace("name = \"cold\"", "name = \"hot\"");
    assert_eq!(rule(&text, None), RuleError::DuplicateName { name: "hot".into() });

    let text = MINIMAL.replace("jeed.test.cold", "jeed.test.hot");
    assert_eq!(
        rule(&text, None),
        RuleError::DuplicateRing { ring: "jeed.test.hot".into(), a: "hot".into(), b: "cold".into() }
    );

    let text = MINIMAL.replace("jeed.test.hot", "jeed/test/hot");
    assert!(matches!(rule(&text, None), RuleError::Ring { feed, .. } if feed == "hot"));

    let text = MINIMAL.replace("name = \"cold\"", "name = \"\"");
    assert_eq!(rule(&text, None), RuleError::EmptyName { index: 1 });
}

#[test]
fn ring_slots_must_be_a_power_of_two() {
    let text = MINIMAL.replace("ring_slots = 4096", "ring_slots = 4000");
    assert_eq!(rule(&text, None), RuleError::RingSlots { feed: "hot".into(), slots: 4000 });
    let text = MINIMAL.replace("ring_slots = 256", "ring_slots = 0");
    assert_eq!(rule(&text, None), RuleError::RingSlots { feed: "cold".into(), slots: 0 });
}

#[test]
fn cores_are_present_distinct_and_real() {
    let text = MINIMAL.replace("cores = [4, 5]", "cores = []");
    assert_eq!(rule(&text, None), RuleError::NoCores { feed: "cold".into() });

    let text = MINIMAL.replace("cores = [2]", "cores = [2, 3]");
    assert_eq!(rule(&text, None), RuleError::SpinOnSeveralCores { feed: "hot".into(), cores: 2 });

    let text = MINIMAL.replace("cores = [4, 5]", "cores = [2, 5]");
    assert_eq!(rule(&text, None), RuleError::CoreShared { core: 2, a: "hot".into(), b: "cold".into() });

    // Without a topology, anything under 64 passes; with one, the box's count
    // is the limit.
    let text = MINIMAL.replace("cores = [4, 5]", "cores = [9]");
    conf(&text).validate(None).unwrap();
    assert_eq!(
        rule(&text, Some(&eight_flat())),
        RuleError::CoreOutOfRange { feed: "cold".into(), core: 9, logical: 8 }
    );
    let text = MINIMAL.replace("cores = [4, 5]", "cores = [64]");
    assert_eq!(
        rule(&text, None),
        RuleError::CoreOutOfRange { feed: "cold".into(), core: 64, logical: 64 }
    );
}

#[test]
fn a_spinning_core_keeps_its_smt_sibling_empty() {
    // Topology: (0,1) and (2,3) are SMT pairs. hot spins on 2; cold on 3
    // would share its execution units.
    let text = MINIMAL.replace("cores = [4, 5]", "cores = [3]");
    assert_eq!(
        rule(&text, Some(&two_cores_smt())),
        RuleError::SiblingShared { spinning: "hot".into(), core: 2, sibling: 3, other: "cold".into() }
    );
    // cold on the other physical core is fine.
    let text = MINIMAL.replace("cores = [4, 5]", "cores = [0, 1]");
    conf(&text).validate(Some(&two_cores_smt())).unwrap();
    // And without a topology the rule cannot be applied, so it is not.
    let text = MINIMAL.replace("cores = [4, 5]", "cores = [3]");
    conf(&text).validate(None).unwrap();
}

#[test]
fn two_blocking_feeds_may_share_a_physical_core() {
    // The sibling rule protects a *spinning* thread. Two parked threads on
    // one core cost nothing.
    let text = MINIMAL.replace("mode = \"spin\"", "mode = \"block\"").replace("cores = [4, 5]", "cores = [3]");
    conf(&text).validate(Some(&two_cores_smt())).unwrap();
}

#[test]
fn one_port_one_socket_across_the_whole_conf() {
    // On Windows a socket binds to INADDR_ANY and receives every group on its
    // port — two feeds on one port would publish everything twice, to two
    // rings. That is worse than the same fault inside one feed, which
    // `Receiver::new` also refuses.
    let text = MINIMAL.replace("239.255.77.93:30315", "239.255.77.93:30302");
    assert_eq!(rule(&text, None), RuleError::PortShared { port: 30302, a: "hot".into(), b: "cold".into() });

    let text = MINIMAL.replace(
        "sockets = [\"239.255.77.92:30302\"]",
        "sockets = [\"239.255.77.92:30302\", \"239.255.77.96:30302\"]",
    );
    assert_eq!(rule(&text, None), RuleError::PortShared { port: 30302, a: "hot".into(), b: "hot".into() });

    let text = MINIMAL.replace("sockets = [\"239.255.77.93:30315\"]", "sockets = []");
    assert_eq!(rule(&text, None), RuleError::NoSockets { feed: "cold".into() });
}

#[test]
fn trcodes_are_present_and_distinct_within_a_feed() {
    let text = MINIMAL.replace("trcodes = [\"M401F\"]", "trcodes = []");
    assert_eq!(rule(&text, None), RuleError::NoTrcodes { feed: "cold".into() });

    let text = MINIMAL.replace("\"G701F\"", "\"B601F\"");
    assert_eq!(rule(&text, None), RuleError::DuplicateTrcode { feed: "hot".into(), code: code("B601F") });
}

// ── warnings ────────────────────────────────────────────────────────────

fn table_with(codes: &[&str]) -> TrCodeTable {
    let mut text = String::from("[code]\n");
    for c in codes {
        text.push_str(&format!("{c} = {{ interface = \"X\" }}\n"));
    }
    TrCodeTable::from_table(&parse(&text).unwrap()).unwrap()
}

#[test]
fn a_code_the_standard_does_not_list_is_a_warning_not_an_error() {
    // 17F arrived on the circuit before it appeared in the standard.
    let c = conf(MINIMAL);
    let table = table_with(&["B601F", "G701F"]);
    let w = c.warnings(Some(&table), Some(&eight_flat()));
    assert_eq!(w, [Warning::NotInTable { feed: "cold".into(), code: code("M401F") }]);
    assert_eq!(w[0].to_string(), "feed[cold]: trcode M401F is in neither standard's table");
}

#[test]
fn a_code_without_a_decoder_is_a_warning() {
    let text = MINIMAL.replace("trcodes = [\"M401F\"]", "trcodes = [\"M401F\", \"A001F\"]");
    let w = conf(&text).warnings(None, Some(&eight_flat()));
    assert_eq!(w, [Warning::NoDecoder { feed: "cold".into(), code: code("A001F") }]);
}

#[test]
fn a_code_under_two_feeds_is_a_warning() {
    let text = MINIMAL.replace("trcodes = [\"M401F\"]", "trcodes = [\"M401F\", \"B601F\"]");
    let w = conf(&text).warnings(None, Some(&eight_flat()));
    assert_eq!(w, [Warning::TrcodeInTwoFeeds { code: code("B601F"), a: "hot".into(), b: "cold".into() }]);
}

#[test]
fn a_missing_topology_is_said() {
    let w = conf(MINIMAL).warnings(None, None);
    assert_eq!(w, [Warning::NoTopology]);
}

// ── the real files ──────────────────────────────────────────────────────

fn repo(rel: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(rel)
}

#[test]
fn the_example_conf_reads_and_fails_validation_only_on_its_blank_sockets() {
    // The template deliberately ships with the circuit assignment commented
    // out; everything else in it must already be a valid conf.
    let c = KrxConf::load(&repo("conf/krx.example.toml")).unwrap();
    assert_eq!(c.feeds.len(), 2);
    assert_eq!(c.feeds[0].cores, [2]);
    assert_eq!(c.feeds[1].cores, [4, 5, 6, 7]);
    assert_eq!(c.validate(None).unwrap_err(), RuleError::NoSockets { feed: "hot".into() });

    // Every trcode the template names is in the generated table. Two have no
    // decoder yet — the 파생 종목정보 master (`A0`) is joined so that the
    // question of whether it carries the 분배그룹번호 can be answered from the
    // counters (`documents/todo.md` §10); it is not published until a
    // decoder exists.
    let table = TrCodeTable::load(&repo("conf/krx_trcodes.toml")).unwrap();
    let w = c.warnings(Some(&table), Some(&eight_flat()));
    assert_eq!(
        w,
        [
            Warning::NoDecoder { feed: "cold".into(), code: code("A001F") },
            Warning::NoDecoder { feed: "cold".into(), code: code("A003F") },
        ]
    );
}

#[test]
fn a_missing_file_is_reported_by_path() {
    let e = KrxConf::load(&repo("conf/does-not-exist.toml")).unwrap_err();
    assert!(matches!(e, ConfError::Io { .. }));
    assert!(e.to_string().contains("does-not-exist.toml"), "{e}");
}
