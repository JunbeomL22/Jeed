//! `jeed_krx::recv::filter`.

use jeed_krx::TrCode;
use jeed_krx::recv::{IsinFilter, TrCodeFilter, parse_isin_list};

fn code(s: &str) -> TrCode {
    TrCode::from_message(s.as_bytes()).unwrap()
}

#[test]
fn an_empty_trcode_set_keeps_nothing() {
    // The asymmetry with `IsinFilter` is deliberate: a conf typo that emptied
    // `trcodes` should produce a silent feed, which is noticed, rather than a
    // ring carrying every data class on the port, which looks like health.
    let f = TrCodeFilter::none();
    assert!(!f.keeps(code("B601F")));
    assert!(f.is_empty());
}

#[test]
fn only_the_listed_codes_are_kept() {
    let f = TrCodeFilter::new([code("B601F"), code("G701F"), code("V101F")]);
    assert!(f.keeps(code("B601F")));
    assert!(f.keeps(code("V101F")));
    // Same data class, different product group — the dispatch key is all five
    // bytes, and so is the filter key.
    assert!(!f.keeps(code("B603F")));
    // Same product group, different data class: the rest of the port's traffic.
    assert!(!f.keeps(code("A301F")));
}

#[test]
fn duplicates_collapse() {
    let f = TrCodeFilter::new([code("B601F"), code("B601F"), code("G701F")]);
    assert_eq!(f.len(), 2);
}

#[test]
fn the_index_addresses_the_same_code_every_time() {
    // The per-socket wiring counters key off this index, so it has to be a
    // stable name for a code and not just a yes/no.
    let f = TrCodeFilter::new([code("G701F"), code("B601F"), code("V101F")]);
    for c in ["B601F", "G701F", "V101F"] {
        let i = f.index_of(code(c)).unwrap();
        assert_eq!(f.code(i), Some(code(c)));
    }
}

#[test]
fn an_empty_isin_set_keeps_everything() {
    let f = IsinFilter::all();
    assert!(f.allows(b"KR4101V90009"));
    assert!(f.allows(b"KR7005930003"));
}

#[test]
fn a_listed_isin_set_keeps_only_what_it_lists() {
    let f = IsinFilter::new([*b"KR4101V90009"]);
    assert!(f.allows(b"KR4101V90009"));
    assert!(!f.allows(b"KR4201V90007"));
}

#[test]
fn a_field_of_the_wrong_width_is_not_an_isin() {
    let f = IsinFilter::new([*b"KR4101V90009"]);
    assert!(!f.allows(b"KR4101V9000"), "eleven bytes is not a 종목코드");
    assert!(!f.allows(b"KR4101V900099"));
}

#[test]
fn an_allow_list_file_ignores_comments_and_blanks() {
    let text = "\
# 2026-07-31 KOSPI200
KR4101V90009

KR4201V90007   # 코스닥150
";
    let f = parse_isin_list(text).unwrap();
    assert_eq!(f.len(), 2);
    assert!(f.allows(b"KR4101V90009"));
    assert!(f.allows(b"KR4201V90007"));
}

#[test]
fn an_allow_list_names_the_line_it_could_not_read() {
    let err = parse_isin_list("KR4101V90009\nKR410\n").unwrap_err();
    assert_eq!(err.line, 2);
    assert_eq!(err.len, 5);
}
