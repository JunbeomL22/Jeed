//! `jeed::toml` — the subset reader, against hand-written documents and the
//! two real files it exists for.

use jeed::toml::{ErrorKind, Table, Value, parse};

fn get<'a>(t: &'a Table, path: &str) -> &'a Value {
    let mut cur: Option<&Value> = None;
    let mut table = t;
    for key in path.split('.') {
        let v = table.get(key).unwrap_or_else(|| panic!("no key `{key}` in `{path}`"));
        cur = Some(v);
        if let Value::Table(inner) = v {
            table = inner;
        }
    }
    cur.expect("non-empty path")
}

// ── values ──────────────────────────────────────────────────────────────

#[test]
fn every_scalar_kind() {
    let t = parse(
        r#"
        s = "basic"
        l = 'literal \n stays'
        i = 42
        neg = -7
        under = 1_000_000
        hex = 0xff
        oct = 0o17
        bin = 0b101
        f = 1.5
        fe = 2e3
        yes = true
        no = false
        "#,
    )
    .unwrap();
    assert_eq!(get(&t, "s").as_str(), Some("basic"));
    assert_eq!(get(&t, "l").as_str(), Some(r"literal \n stays"));
    assert_eq!(get(&t, "i").as_integer(), Some(42));
    assert_eq!(get(&t, "neg").as_integer(), Some(-7));
    assert_eq!(get(&t, "under").as_integer(), Some(1_000_000));
    assert_eq!(get(&t, "hex").as_integer(), Some(255));
    assert_eq!(get(&t, "oct").as_integer(), Some(15));
    assert_eq!(get(&t, "bin").as_integer(), Some(5));
    assert_eq!(get(&t, "f").as_float(), Some(1.5));
    assert_eq!(get(&t, "fe").as_float(), Some(2000.0));
    assert_eq!(get(&t, "yes").as_bool(), Some(true));
    assert_eq!(get(&t, "no").as_bool(), Some(false));
}

#[test]
fn an_integer_is_not_a_float() {
    // `1` where `1.0` was meant should be said, not promoted.
    let t = parse("x = 1").unwrap();
    assert_eq!(get(&t, "x").as_float(), None);
    assert_eq!(get(&t, "x").type_name(), "integer");
}

#[test]
fn basic_string_escapes() {
    let t = parse(r#"s = "tab\there \"quoted\" back\\slash é \U0001F600""#).unwrap();
    assert_eq!(get(&t, "s").as_str(), Some("tab\there \"quoted\" back\\slash é 😀"));
}

#[test]
fn korean_passes_through_both_string_kinds() {
    let t = parse(
        r#"
        a = "코스피200선물"
        b = '파생A'
        "01F" = "kospi200선물"
        "#,
    )
    .unwrap();
    assert_eq!(get(&t, "a").as_str(), Some("코스피200선물"));
    assert_eq!(get(&t, "b").as_str(), Some("파생A"));
    assert_eq!(get(&t, "01F").as_str(), Some("kospi200선물"));
}

#[test]
fn arrays_span_lines_and_carry_comments() {
    let t = parse(
        r#"
        sockets = [
          # "233.xxx.xxx.92:10302",   # commented out
          "239.255.77.92:30302",
          "239.255.77.96:30322",      # trailing comment
        ]
        cores = [4, 5, 6, 7]
        empty = []
        nested = [[1, 2], ["a"]]
        "#,
    )
    .unwrap();
    let sockets = get(&t, "sockets").as_array().unwrap();
    assert_eq!(sockets.len(), 2);
    assert_eq!(sockets[1].as_str(), Some("239.255.77.96:30322"));
    let cores: Vec<i64> = get(&t, "cores").as_array().unwrap().iter().map(|v| v.as_integer().unwrap()).collect();
    assert_eq!(cores, [4, 5, 6, 7]);
    assert!(get(&t, "empty").as_array().unwrap().is_empty());
    let nested = get(&t, "nested").as_array().unwrap();
    assert_eq!(nested[0].as_array().unwrap().len(), 2);
    assert_eq!(nested[1].as_array().unwrap()[0].as_str(), Some("a"));
}

#[test]
fn inline_tables_nest() {
    let t = parse(
        r#"
        G701F = { interface = "IFMSRPD0037", group = "01F", also = ["IFMSRID0007"] }
        B201S = { group = "01S", by = { "주식파생" = "IFMSRPD0062", "증권A" = "IFMSRPD0021" } }
        empty = {}
        "#,
    )
    .unwrap();
    assert_eq!(get(&t, "G701F.interface").as_str(), Some("IFMSRPD0037"));
    assert_eq!(get(&t, "G701F.also").as_array().unwrap()[0].as_str(), Some("IFMSRID0007"));
    assert_eq!(get(&t, "B201S.by.증권A").as_str(), Some("IFMSRPD0021"));
    assert!(get(&t, "empty").as_table().unwrap().is_empty());
}

// ── keys and headers ────────────────────────────────────────────────────

#[test]
fn dotted_keys_build_tables() {
    let t = parse("a.b.c = 1\na.b.d = 2\na.e = 3").unwrap();
    assert_eq!(get(&t, "a.b.c").as_integer(), Some(1));
    assert_eq!(get(&t, "a.b.d").as_integer(), Some(2));
    assert_eq!(get(&t, "a.e").as_integer(), Some(3));
    assert_eq!(get(&t, "a").as_table().unwrap().len(), 2);
}

#[test]
fn headers_open_tables_and_dotted_headers_nest() {
    let t = parse(
        r#"
        top = 0
        [health]
        heartbeat_ms = 100
        [a.b]
        x = 1
        [a]
        y = 2
        "#,
    )
    .unwrap();
    assert_eq!(get(&t, "top").as_integer(), Some(0));
    assert_eq!(get(&t, "health.heartbeat_ms").as_integer(), Some(100));
    assert_eq!(get(&t, "a.b.x").as_integer(), Some(1));
    // `[a]` after `[a.b]` defines the implicitly created parent — allowed.
    assert_eq!(get(&t, "a.y").as_integer(), Some(2));
}

#[test]
fn array_of_tables_accumulates_in_order() {
    let t = parse(
        r#"
        [[feed]]
        name = "hot"
        [[feed]]
        name = "cold"
        cores = [4]
        [feed.extra]
        deep = true
        "#,
    )
    .unwrap();
    let feeds = get(&t, "feed").as_array().unwrap();
    assert_eq!(feeds.len(), 2);
    assert_eq!(feeds[0].as_table().unwrap().get("name").unwrap().as_str(), Some("hot"));
    let cold = feeds[1].as_table().unwrap();
    assert_eq!(cold.get("name").unwrap().as_str(), Some("cold"));
    // A `[feed.x]` header after `[[feed]]` refers to the last element.
    assert_eq!(cold.get("extra").unwrap().as_table().unwrap().get("deep").unwrap().as_bool(), Some(true));
}

#[test]
fn entries_keep_file_order() {
    let t = parse("z = 1\na = 2\nm = 3").unwrap();
    let keys: Vec<&str> = t.keys().collect();
    assert_eq!(keys, ["z", "a", "m"]);
}

#[test]
fn crlf_line_endings_and_blank_lines() {
    let t = parse("a = 1\r\n\r\n# comment\r\n[t]\r\nb = 2\r\n").unwrap();
    assert_eq!(get(&t, "a").as_integer(), Some(1));
    assert_eq!(get(&t, "t.b").as_integer(), Some(2));
}

#[test]
fn an_empty_document_is_an_empty_table() {
    assert!(parse("").unwrap().is_empty());
    assert!(parse("# only a comment\n\n").unwrap().is_empty());
}

// ── errors ──────────────────────────────────────────────────────────────

fn kind(text: &str) -> ErrorKind {
    parse(text).unwrap_err().kind
}

#[test]
fn a_key_assigned_twice_is_refused() {
    assert_eq!(kind("a = 1\na = 2"), ErrorKind::Duplicate("a".into()));
    assert_eq!(kind("[t]\nx = 1\n[t]\ny = 2"), ErrorKind::Duplicate("t".into()));
    assert_eq!(kind("t = { a = 1, a = 2 }"), ErrorKind::Duplicate("a".into()));
}

#[test]
fn a_value_cannot_become_a_table() {
    assert_eq!(kind("a = 1\n[a]\nb = 2"), ErrorKind::NotATable("a".into()));
    assert_eq!(kind("a = 1\na.b = 2"), ErrorKind::NotATable("a".into()));
    assert_eq!(kind("[a]\nx = 1\n[[a]]\ny = 2"), ErrorKind::NotATable("a".into()));
}

#[test]
fn errors_carry_the_line_number() {
    let e = parse("a = 1\nb = 2\nc = \"open").unwrap_err();
    assert_eq!(e.line, 3);
    assert_eq!(e.kind, ErrorKind::Unterminated("string"));
    assert_eq!(e.to_string(), "line 3: unterminated string");
}

#[test]
fn what_is_not_implemented_is_named() {
    assert_eq!(kind("s = \"\"\"multi\"\"\""), ErrorKind::Unsupported("a multi-line string"));
    assert_eq!(kind("s = '''multi'''"), ErrorKind::Unsupported("a multi-line literal string"));
    assert_eq!(kind("d = 2026-09-13"), ErrorKind::Unsupported("a date or time"));
    assert_eq!(kind("t = 07:32:00"), ErrorKind::Unsupported("a date or time"));
    assert_eq!(kind("x = inf"), ErrorKind::Unsupported("a non-finite float"));
}

#[test]
fn malformed_statements() {
    assert_eq!(kind("a 1"), ErrorKind::Expected("`=` after key"));
    assert_eq!(kind("a = 1 b = 2"), ErrorKind::Expected("end of line"));
    assert_eq!(kind("a ="), ErrorKind::Expected("a value"));
    assert_eq!(kind("[t\nx = 1"), ErrorKind::Expected("`]` after table name"));
    assert_eq!(kind("a = [1, 2"), ErrorKind::Unterminated("array"));
    assert_eq!(kind("a = { x = 1"), ErrorKind::Expected("`,` or `}` in inline table"));
    assert_eq!(kind("a = \"bad \\q escape\""), ErrorKind::BadEscape);
    assert_eq!(kind("a = 12abc"), ErrorKind::BadNumber("12abc".into()));
    assert_eq!(kind("a = 0x"), ErrorKind::BadNumber("0x".into()));
}

// ── the real files ──────────────────────────────────────────────────────

fn repo_file(rel: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn the_example_conf_reads_as_written() {
    let t = parse(&repo_file("conf/krx.example.toml")).unwrap();
    assert_eq!(get(&t, "trcode_table").as_str(), Some("conf/krx_trcodes.toml"));
    let feeds = get(&t, "feed").as_array().unwrap();
    assert_eq!(feeds.len(), 2);
    let hot = feeds[0].as_table().unwrap();
    assert_eq!(hot.get("mode").unwrap().as_str(), Some("spin"));
    assert_eq!(hot.get("cores").unwrap().as_array().unwrap()[0].as_integer(), Some(2));
    assert_eq!(hot.get("trcodes").unwrap().as_array().unwrap().len(), 8);
    // The template ships with every socket commented out.
    assert!(hot.get("sockets").unwrap().as_array().unwrap().is_empty());
    assert_eq!(get(&t, "health.stale_ms").as_integer(), Some(500));
}

#[test]
fn the_generated_trcode_table_reads_as_written() {
    let t = parse(&repo_file("conf/krx_trcodes.toml")).unwrap();
    assert_eq!(get(&t, "spec_version").as_str(), Some("1.341"));
    let declared = get(&t, "code_count").as_integer().unwrap() as usize;
    let code = get(&t, "code").as_table().unwrap();
    assert_eq!(code.len(), declared, "[code] has as many entries as the header says");
    assert_eq!(get(&t, "code.G701F.interface").as_str(), Some("IFMSRPD0037"));
    assert_eq!(get(&t, "code.H101F.also").as_array().unwrap()[0].as_str(), Some("IFMSRID0007"));
    assert_eq!(get(&t, "code_by_channel.B201S.by.증권A").as_str(), Some("IFMSRPD0021"));
    assert_eq!(get(&t, "product_group.01F").as_str(), Some("kospi200선물"));
    assert_eq!(get(&t, "interface.IFMSRPD0037.length").as_integer(), Some(431));
}
