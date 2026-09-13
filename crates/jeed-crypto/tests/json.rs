//! `jeed_crypto::json` — the scanner, on its own.
//!
//! The decoders exercise it against real frames; these are the edges the
//! frames do not reach, and the assumptions the decoders are allowed to make.

use jeed_crypto::json::{
    next_field, next_key, object_at, objects_at, parse_bool, parse_scalar_bytes,
    parse_scalar_u64, parse_string_bytes, parse_u64, quote_levels, skip_value, skip_ws,
};
use jeed_crypto::{CryptoError, Instrument};
use jeed_wire::{Venue, WIRE_MAX_DEPTH, WireLevel};

fn inst() -> Instrument {
    Instrument::new(Venue::BinanceSpot, b"BTCUSDT", 2, 5).unwrap()
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

#[test]
fn keys_come_back_as_their_first_byte_and_the_match_is_case_sensitive() {
    // The whole dispatch rests on this: `e`/`E`, `u`/`U`, `b`/`B` are the
    // pairs Binance actually uses, and they must not collapse.
    let data = br#"{"e":1,"E":2,"u":3,"U":4}"#;
    let mut pos = 0;
    let mut seen = Vec::new();
    loop {
        let (key, next) = next_key(data, pos);
        pos = next;
        if key == 0 {
            break;
        }
        let (v, next) = parse_u64(data, pos);
        pos = next;
        seen.push((key, v));
    }
    assert_eq!(seen, vec![(b'e', 1), (b'E', 2), (b'u', 3), (b'U', 4)]);
}

#[test]
fn a_two_character_key_reports_its_first_byte() {
    // `pu` is matched as `p`, which is what the USD-M delta decoder relies on.
    let (key, pos) = next_key(br#"{"pu":390497794}"#, 0);
    assert_eq!(key, b'p');
    assert_eq!(parse_u64(br#"{"pu":390497794}"#, pos).0, 390_497_794);
}

#[test]
fn an_exhausted_object_reports_key_zero() {
    let data = br#"{"u":1}"#;
    let (_, pos) = next_key(data, 0);
    let pos = parse_u64(data, pos).1;
    assert_eq!(next_key(data, pos).0, 0);
}

#[test]
fn whitespace_between_tokens_is_ignored() {
    let data = b"{ \"u\" : \t 42 }";
    let (key, pos) = next_key(data, 0);
    assert_eq!(key, b'u');
    assert_eq!(parse_u64(data, pos).0, 42);
    assert_eq!(skip_ws(b"   x", 0), 3);
}

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

#[test]
fn strings_are_borrowed_not_copied() {
    let data = br#"{"s":"BTCUSDT"}"#;
    let (_, pos) = next_key(data, 0);
    let (s, _) = parse_string_bytes(data, pos);
    assert_eq!(s, b"BTCUSDT");
    // A slice into the frame, so nothing was allocated to produce it.
    assert!(std::ptr::eq(s.as_ptr(), data[6..].as_ptr()));
}

#[test]
fn an_escaped_quote_does_not_end_a_string_early() {
    let data = br#"{"x":"a\"b","u":7}"#;
    let (_, pos) = next_key(data, 0);
    let (s, pos) = parse_string_bytes(data, pos);
    assert_eq!(s, br#"a\"b"#, "returned raw, escapes and all");
    assert_eq!(next_key(data, pos).0, b'u', "and the scan stayed in step");
}

#[test]
fn booleans_read_both_ways() {
    let data = br#"{"m":true,"M":false,"u":9}"#;
    let (_, pos) = next_key(data, 0);
    let (m, pos) = parse_bool(data, pos);
    assert!(m);
    let (_, pos) = next_key(data, pos);
    let (big_m, pos) = parse_bool(data, pos);
    assert!(!big_m);
    assert_eq!(next_key(data, pos).0, b'u');
}

#[test]
fn a_wild_number_saturates_instead_of_wrapping() {
    // A wrapped millisecond field would come out the far side as a plausible
    // timestamp; a saturated one is visibly nonsense.
    let (v, _) = parse_u64(b"999999999999999999999999", 0);
    assert_eq!(v, u64::MAX);
}

// ---------------------------------------------------------------------------
// Skipping
// ---------------------------------------------------------------------------

#[test]
fn skip_value_steps_over_every_kind_of_value() {
    for (frame, name) in [
        (&br#"{"x":{"a":[1,2],"b":"}"},"u":5}"#[..], "object with a brace in a string"),
        (&br#"{"x":[[1,"]"],[2]],"u":5}"#[..], "array with a bracket in a string"),
        (&br#"{"x":"plain","u":5}"#[..], "string"),
        (&br#"{"x":null,"u":5}"#[..], "null"),
        (&br#"{"x":-1.5e3,"u":5}"#[..], "number"),
    ] {
        let (_, pos) = next_key(frame, 0);
        let pos = skip_value(frame, pos);
        let (key, pos) = next_key(frame, pos);
        assert_eq!(key, b'u', "{name}");
        assert_eq!(parse_u64(frame, pos).0, 5, "{name}");
    }
}

// ---------------------------------------------------------------------------
// Level arrays
// ---------------------------------------------------------------------------

#[test]
fn a_levels_array_must_be_where_the_value_is() {
    // Scanning forward for the next `[` would let an absent `bids` quietly
    // adopt the `asks` array that follows it.
    let inst = inst();
    let data = br#"{"bids":null,"asks":[["4.00","1.00000"]]}"#;
    let (_, pos) = next_key(data, 0);
    let mut out = [WireLevel::default(); WIRE_MAX_DEPTH];
    let (depth, _) = quote_levels(&inst, data, pos, &mut out).unwrap();
    assert_eq!(depth, 0);
    assert_eq!(out[0], WireLevel::default());
}

#[test]
fn a_third_element_in_a_level_does_not_become_the_next_level() {
    // Binance used to send `[price, qty, []]` on some endpoints.
    let inst = inst();
    let data = br#"{"bids":[["4.00","1.00000",[]],["3.99","2.00000",[]]]}"#;
    let (_, pos) = next_key(data, 0);
    let mut out = [WireLevel::default(); WIRE_MAX_DEPTH];
    let (depth, _) = quote_levels(&inst, data, pos, &mut out).unwrap();

    assert_eq!(depth, 2);
    assert_eq!(out[0], WireLevel::new(400, 100_000));
    assert_eq!(out[1], WireLevel::new(399, 200_000));
}

#[test]
fn an_empty_array_is_a_side_with_no_levels() {
    let inst = inst();
    let data = br#"{"bids":[]}"#;
    let (_, pos) = next_key(data, 0);
    let mut out = [WireLevel::default(); WIRE_MAX_DEPTH];
    assert_eq!(quote_levels(&inst, data, pos, &mut out).unwrap().0, 0);
}

#[test]
fn a_level_that_will_not_parse_stops_the_side() {
    let inst = inst();
    let data = br#"{"bids":[["4.00","1.00000"],["oops","1.00000"]]}"#;
    let (_, pos) = next_key(data, 0);
    let mut out = [WireLevel::default(); WIRE_MAX_DEPTH];
    assert!(matches!(
        quote_levels(&inst, data, pos, &mut out),
        Err(CryptoError::Field { key: "level price", .. })
    ));
}

// ---------------------------------------------------------------------------
// Whole keys
// ---------------------------------------------------------------------------

#[test]
fn next_field_tells_apart_keys_that_share_a_first_byte() {
    // The reason it exists: Upbit's unit object has four keys and two pairs of
    // them collide on the first byte, in both spellings.
    let data = br#"{"ask_price":1.0,"ask_size":2.0,"bid_price":3.0,"bid_size":4.0}"#;

    let mut pos = 0;
    let mut keys = Vec::new();
    loop {
        let (key, next) = next_field(data, pos);
        if key.is_empty() {
            break;
        }
        keys.push(key.to_vec());
        pos = skip_value(data, next);
    }

    assert_eq!(
        keys,
        vec![
            b"ask_price".to_vec(),
            b"ask_size".to_vec(),
            b"bid_price".to_vec(),
            b"bid_size".to_vec()
        ]
    );
}

#[test]
fn next_field_reports_an_empty_key_when_the_object_is_spent() {
    let data = br#"{"a":1}"#;
    let (_, after) = next_field(data, 0);
    let (key, _) = next_field(data, skip_value(data, after));
    assert!(key.is_empty());
}

#[test]
fn next_field_lands_on_the_value_past_the_colon_and_its_whitespace() {
    let data = br#"{ "code" :  "KRW-BTC" }"#;
    let (key, value) = next_field(data, 0);
    assert_eq!(key, b"code");
    assert_eq!(data[value], b'"');
    assert_eq!(parse_scalar_bytes(data, value).0, b"KRW-BTC");
}

// ---------------------------------------------------------------------------
// Scalars, quoted or not
// ---------------------------------------------------------------------------

#[test]
fn a_scalar_reads_the_same_digits_whether_or_not_it_is_quoted() {
    // Binance, OKX and Bybit quote their numbers; Upbit does not. Both have to
    // reach the extractor as the same bytes.
    let quoted = br#"{"p":"152430000.0"}"#;
    let bare = br#"{"p":152430000.0}"#;

    let (_, v) = next_field(quoted, 0);
    assert_eq!(parse_scalar_bytes(quoted, v).0, b"152430000.0");
    let (_, v) = next_field(bare, 0);
    assert_eq!(parse_scalar_bytes(bare, v).0, b"152430000.0");
}

#[test]
fn a_scalar_stops_where_the_value_stops() {
    // The position it hands back is the whole point: an unquoted number that
    // swallowed the comma, or a quoted one that stopped before its closing
    // quote, would leave the next key unreadable.
    let data = br#"{"a":1.5,"b":"2.5","c":3}"#;

    let (_, v) = next_field(data, 0);
    let (_, after_a) = parse_scalar_bytes(data, v);
    let (key, v) = next_field(data, after_a);
    assert_eq!(key, b"b");

    let (_, after_b) = parse_scalar_bytes(data, v);
    let (key, _) = next_field(data, after_b);
    assert_eq!(key, b"c");
}

#[test]
fn null_reads_as_nothing_rather_than_as_zero() {
    // An empty slice fails the number readers, which is right for a field the
    // decoder insisted on — a zero price would not be.
    let data = br#"{"p":null,"q":1}"#;
    let (_, v) = next_field(data, 0);
    let (bytes, after) = parse_scalar_bytes(data, v);

    assert!(bytes.is_empty());
    assert_eq!(next_field(data, after).0, b"q");
}

#[test]
fn a_quoted_integer_does_not_leave_its_closing_quote_behind() {
    // `parse_u64` stops on the first non-digit, which for a quoted value is
    // the closing quote — and the next scan would read that quote as the
    // opening one of a key. OKX quotes `ts` and leaves `seqId` bare, in one
    // object.
    let data = br#"{"ts":"1706000000000","seqId":1234567890}"#;

    let (_, v) = next_field(data, 0);
    assert_eq!(parse_u64(data, v).0, 1_706_000_000_000);
    assert_eq!(next_field(data, parse_u64(data, v).1).0, b",", "the trap");

    let (value, after) = parse_scalar_u64(data, v);
    assert_eq!(value, 1_706_000_000_000);
    let (key, v) = next_field(data, after);
    assert_eq!(key, b"seqId");
    assert_eq!(parse_scalar_u64(data, v).0, 1_234_567_890);
}

// ---------------------------------------------------------------------------
// Envelopes
// ---------------------------------------------------------------------------

#[test]
fn an_object_comes_back_without_its_braces_and_cannot_run_past_them() {
    // That is the property the flat scanners need: walking the inner slice has
    // nowhere to wander to.
    let data = br#"{"topic":"x","data":{"s":"BTCUSDT","u":7},"cts":1}"#;
    let (_, v) = next_field(data, 0);
    let (_, after_topic) = parse_scalar_bytes(data, v);
    let (key, v) = next_field(data, after_topic);
    assert_eq!(key, b"data");

    let (inner, past) = object_at(data, v).unwrap();
    assert_eq!(inner, br#""s":"BTCUSDT","u":7"#);
    assert_eq!(next_field(data, past).0, b"cts");

    // And the inner slice ends where the object does.
    let mut pos = 0;
    let mut keys = Vec::new();
    loop {
        let (key, next) = next_field(inner, pos);
        if key.is_empty() {
            break;
        }
        keys.push(key.to_vec());
        pos = skip_value(inner, next);
    }
    assert_eq!(keys, vec![b"s".to_vec(), b"u".to_vec()]);
}

#[test]
fn object_at_refuses_anything_that_is_not_an_object_here() {
    assert!(object_at(br#"[1,2]"#, 0).is_none());
    assert!(object_at(br#"  "x""#, 0).is_none());
    assert!(object_at(br#"{"a":1"#, 0).is_none(), "an object that never closes");
    assert!(object_at(br#"   {"a":1}"#, 0).is_some(), "leading whitespace is fine");
}

#[test]
fn every_object_in_an_array_is_handed_out() {
    // A trade frame is a batch, and taking only the last one would drop prints
    // that moved the tape.
    let data = br#"[{"i":1},{"i":2},{"i":3}]"#;
    let seen: Vec<&[u8]> = objects_at(data, 0).unwrap().collect();

    assert_eq!(seen, vec![&b"\"i\":1"[..], &b"\"i\":2"[..], &b"\"i\":3"[..]]);
}

#[test]
fn an_empty_array_hands_out_nothing() {
    assert_eq!(objects_at(br#"[]"#, 0).unwrap().count(), 0);
    assert_eq!(objects_at(br#"[  ]"#, 0).unwrap().count(), 0);
}

#[test]
fn a_brace_inside_a_string_does_not_end_an_object() {
    let data = br#"[{"i":"}{","j":2}]"#;
    let seen: Vec<&[u8]> = objects_at(data, 0).unwrap().collect();

    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0], br#""i":"}{","j":2"#);
}

#[test]
fn objects_at_requires_the_array_to_be_here() {
    // Scanning forward for the next `[` would silently adopt a later field's
    // array, which is the same rule the level readers follow.
    assert!(objects_at(br#"{"data":[{"i":1}]}"#, 0).is_none());
}

// ---------------------------------------------------------------------------
// Unquoted level arrays
// ---------------------------------------------------------------------------

#[test]
fn a_level_pair_reads_unquoted_numbers_too() {
    // No venue in this crate sends a bare level pair today, but the reader is
    // shared with the scalar path and a comma between two bare numbers is the
    // one place the two spellings behave differently.
    let inst = inst();
    let data = br#"[[119250.01,3.12100],[119250.00,1.00000]]"#;
    let mut out = [WireLevel::default(); WIRE_MAX_DEPTH];
    let (depth, _) = quote_levels(&inst, data, 0, &mut out).unwrap();

    assert_eq!(depth, 2);
    assert_eq!(out[0].price, 11_925_001);
    assert_eq!(out[0].qty, 312_100);
    assert_eq!(out[1].price, 11_925_000);
    assert_eq!(out[1].qty, 100_000);
}

// ---------------------------------------------------------------------------
// Exponent notation
// ---------------------------------------------------------------------------

mod exponent {
    use jeed_crypto::json::{PLAIN_DECIMAL_LEN, plain_decimal};

    fn plain(s: &str) -> Option<String> {
        let mut out = [0u8; PLAIN_DECIMAL_LEN];
        plain_decimal(s.as_bytes(), &mut out).map(|n| String::from_utf8(out[..n].to_vec()).unwrap())
    }

    #[test]
    fn a_number_without_an_exponent_is_left_to_the_caller() {
        assert_eq!(plain("104525000.0"), None);
        assert_eq!(plain("0.00084280"), None);
        assert_eq!(plain(""), None);
    }

    #[test]
    fn upbits_prices_come_back_plain() {
        // Java's Double.toString, above ten million.
        assert_eq!(plain("1.04525E8").as_deref(), Some("104525000"));
        assert_eq!(plain("1.0452512E8").as_deref(), Some("104525120"));
        assert_eq!(plain("1.04525123456E8").as_deref(), Some("104525123.456"));
        assert_eq!(plain("2E7").as_deref(), Some("20000000"));
    }

    #[test]
    fn small_numbers_and_signs_are_moved_not_computed() {
        assert_eq!(plain("2e-7").as_deref(), Some("0.0000002"));
        assert_eq!(plain("8.428E-4").as_deref(), Some("0.0008428"));
        assert_eq!(plain("1.5e0").as_deref(), Some("1.5"));
        assert_eq!(plain("15e-1").as_deref(), Some("1.5"));
        assert_eq!(plain("-1.5E+2").as_deref(), Some("-150"));
        assert_eq!(plain("+2.5E1").as_deref(), Some("25"));
    }

    #[test]
    fn junk_and_absurd_exponents_are_refused() {
        assert_eq!(plain("1.5E"), None);
        assert_eq!(plain("E8"), None);
        assert_eq!(plain("1.5.2E3"), None);
        assert_eq!(plain("1xE3"), None);
        assert_eq!(plain("1E1000"), None);
        assert_eq!(plain("1E99"), None, "a hundred digits is not a price");
        assert_eq!(plain("1E-99"), None);
    }
}
