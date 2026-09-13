//! `jeed_krx::message` — the frame check that runs before any field is read.

use jeed_krx::{END_KEYWORD, KrxError, TrCode, validate, validate_as};

fn frame(len: usize) -> Vec<u8> {
    let mut v = b"B601F".to_vec();
    v.resize(len - 1, b'0');
    v.push(END_KEYWORD);
    v
}

#[test]
fn a_whole_frame_passes() {
    assert_eq!(validate(&frame(324), 324), Ok(()));
}

#[test]
fn a_truncated_datagram_is_refused_before_any_field_is_read() {
    // The point of checking first: a short datagram parsed field-by-field
    // produces plausible-looking numbers all the way to the failure.
    let mut short = frame(324);
    short.truncate(300);
    assert_eq!(validate(&short, 324), Err(KrxError::Length { expected: 324, actual: 300 }));
}

#[test]
fn a_frame_without_the_end_keyword_is_refused() {
    let mut f = frame(324);
    *f.last_mut().unwrap() = b'0';
    assert_eq!(validate(&f, 324), Err(KrxError::EndKeyword { found: b'0' }));
}

#[test]
fn the_length_and_the_end_keyword_are_two_independent_statements() {
    // Same position as FIX's BodyLength and CheckSum: either alone can be
    // satisfied by garbage.
    let mut right_length_wrong_end = frame(324);
    *right_length_wrong_end.last_mut().unwrap() = 0xFE;
    assert!(validate(&right_length_wrong_end, 324).is_err());

    let mut wrong_length_right_end = frame(324);
    wrong_length_right_end.push(END_KEYWORD);
    assert!(validate(&wrong_length_right_end, 324).is_err());
}

#[test]
fn a_decoder_refuses_bytes_that_are_not_its_own_message() {
    // The receive loop dispatched on the trcode, so re-reading it here is the
    // decoder refusing to apply one interface's layout to another's bytes.
    let mut f = frame(324);
    f[..5].copy_from_slice(b"G701F");
    assert_eq!(
        validate_as(&f, TrCode::new(*b"B601F"), 324),
        Err(KrxError::UnknownTrCode { code: TrCode::new(*b"G701F") })
    );
    assert_eq!(validate_as(&f, TrCode::new(*b"G701F"), 324), Ok(()));
}

#[test]
fn an_empty_datagram_is_refused_without_indexing_it() {
    assert_eq!(validate(&[], 0), Err(KrxError::TooShort { need: 6, got: 0 }));
}
