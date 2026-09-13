//! `jeed_krx::trcode` — the five-byte dispatch key.

use jeed_krx::{KrxError, TrCode};

#[test]
fn a_trcode_splits_into_data_class_and_product_group() {
    let c = TrCode::new(*b"B601F");
    assert_eq!(c.data_class(), *b"B6");
    assert_eq!(c.product_group(), *b"01F");
    assert_eq!(c.to_string(), "B601F");
    assert!(c.is_derivative());
}

#[test]
fn the_data_class_alone_does_not_identify_a_layout() {
    // B6 spans eight interfaces from 324 to 1387 bytes. Dispatching on two
    // bytes would apply the derivative layout to a REPO message.
    let derivative = TrCode::new(*b"B601F");
    let equity = TrCode::new(*b"B601S");
    assert_eq!(derivative.data_class(), equity.data_class());
    assert_ne!(derivative, equity);
    assert_ne!(derivative.as_u64(), equity.as_u64());
    assert!(!equity.is_derivative());
}

#[test]
fn packing_into_a_u64_is_lossless() {
    for code in [b"B601F", b"G704F", b"V103F", b"Q216F", b"A301F", b"B703S"] {
        let c = TrCode::new(*code);
        assert_eq!(c.as_u64() & 0xff, code[0] as u64);
        assert_eq!(c.as_u64() >> 32, code[4] as u64);
    }
    // All five bytes participate, so an allow-set of u64 is exact.
    let all: Vec<u64> = [b"B601F", b"B602F", b"B601S", b"C601F", b"B611F"]
        .iter()
        .map(|c| TrCode::new(**c).as_u64())
        .collect();
    let mut sorted = all.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), all.len());
}

#[test]
fn a_trcode_is_read_off_the_front_of_the_datagram() {
    let msg = b"G701F00000001G140KR4101V90009";
    assert_eq!(TrCode::from_message(msg), Ok(TrCode::new(*b"G701F")));
}

#[test]
fn a_datagram_too_short_to_hold_one_is_refused() {
    assert_eq!(
        TrCode::from_message(b"G70"),
        Err(KrxError::TooShort { need: 5, got: 3 })
    );
}

#[test]
fn a_non_printable_code_still_renders() {
    // A stray datagram must be loggable, not a panic.
    let c = TrCode::new([0, 1, 2, 3, 4]);
    assert_eq!(c.as_str(), None);
    assert!(c.to_string().contains("00"));
}

#[test]
fn from_u64_is_the_inverse_of_as_u64() {
    for code in [b"B601F", b"G704F", b"V103F", b"Q216F", b"A301F", b"B703S", b"\x00\x01\x02\x03\x04"] {
        let c = TrCode::new(*code);
        assert_eq!(TrCode::from_u64(c.as_u64()), c);
    }
    // Bytes above the fifth do not belong to a code and are ignored.
    assert_eq!(TrCode::from_u64(TrCode::new(*b"B601F").as_u64() | 0xff << 40), TrCode::new(*b"B601F"));
}
