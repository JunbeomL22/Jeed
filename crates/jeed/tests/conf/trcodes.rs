//! `jeed::conf::trcodes` — the generated table.

use jeed::conf::TrCodeTable;
use jeed::toml::parse;
use jeed_krx::TrCode;

fn code(s: &str) -> TrCode {
    TrCode::from_message(s.as_bytes()).unwrap()
}

#[test]
fn the_generated_table_loads() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conf/krx_trcodes.toml");
    let t = TrCodeTable::load(&path).unwrap();
    assert_eq!(t.spec_version, "1.341");
    assert_eq!(t.channel_spec_version, "1.26");
    assert!(t.len() > 500, "{}", t.len());

    assert!(t.knows(code("B601F")));
    assert!(t.knows(code("G701F")));
    assert_eq!(t.interface_of(code("G701F")), Some("IFMSRPD0037"));

    // Listed only under [code_by_channel]: known, but with no single interface.
    assert!(t.knows(code("B201S")));
    assert_eq!(t.interface_of(code("B201S")), None);

    assert!(!t.knows(code("ZZZZZ")));
}

#[test]
fn a_synthetic_table_reads_the_same_way() {
    let t = TrCodeTable::from_table(
        &parse(
            r#"
            spec_version = "9.9"
            [code]
            B601F = { interface = "IFMSRPD0034", group = "01F" }
            [code_by_channel]
            B201S = { group = "01S", by = { "증권A" = "IFMSRPD0021" } }
            "#,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(t.spec_version, "9.9");
    assert_eq!(t.len(), 1);
    assert!(t.knows(code("B601F")));
    assert!(t.knows(code("B201S")));
    assert!(!t.knows(code("G701F")));
}

#[test]
fn an_empty_document_is_an_empty_table() {
    let t = TrCodeTable::from_table(&parse("").unwrap()).unwrap();
    assert!(t.is_empty());
    assert_eq!(t.spec_version, "");
    assert!(!t.knows(code("B601F")));
}

#[test]
fn a_key_that_is_not_five_bytes_is_refused() {
    let e = TrCodeTable::from_table(&parse("[code]\nB601 = { interface = \"X\" }").unwrap()).unwrap_err();
    assert_eq!(e.to_string(), "code: `B601` is not a five-byte trcode");
}

#[test]
fn codes_enumerates_the_code_section_only() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conf/krx_trcodes.toml");
    let t = TrCodeTable::load(&path).unwrap();
    let codes: Vec<TrCode> = t.codes().collect();
    assert_eq!(codes.len(), t.len());
    assert!(codes.contains(&code("B601F")));
    assert!(codes.contains(&code("G717F")), "the channel standard's additions are in [code]");
    // Known only by channel: not a fixed interface, so not enumerated.
    assert!(!codes.contains(&code("B201S")));
    // Every enumerated code is one the table knows, round-tripped through u64.
    assert!(codes.iter().all(|&c| t.knows(c)));
}
