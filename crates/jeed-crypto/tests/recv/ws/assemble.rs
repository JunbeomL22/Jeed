//! Tests for `src/recv/ws/assemble.rs`.

use jeed_crypto::recv::ws::{Assembly, Opcode, WsError};

#[test]
fn three_fragments_become_one_message() {
    let mut a = Assembly::new(64);
    assert!(!a.is_open());

    a.begin(Opcode::Text, b"{\"a\":").unwrap();
    assert!(a.is_open());
    assert_eq!(a.opcode(), Some(Opcode::Text));
    a.append(b"1,\"b\"").unwrap();
    a.append(b":2}").unwrap();

    let (op, bytes) = a.take().expect("open");
    assert_eq!(op, Opcode::Text);
    assert_eq!(bytes, b"{\"a\":1,\"b\":2}");
    assert!(!a.is_open(), "taken means closed");
}

#[test]
fn take_on_nothing_is_none() {
    let mut a = Assembly::new(8);
    assert!(a.take().is_none());
}

#[test]
fn a_continuation_with_nothing_open_is_a_protocol_error() {
    let mut a = Assembly::new(8);
    assert_eq!(a.append(b"x"), Err(WsError::UnexpectedContinuation));
}

#[test]
fn a_new_message_while_one_is_open_is_a_protocol_error() {
    let mut a = Assembly::new(8);
    a.begin(Opcode::Text, b"ab").unwrap();
    assert_eq!(a.begin(Opcode::Text, b"cd"), Err(WsError::InterleavedMessage));
    assert_eq!(a.bytes(), b"ab", "the open message is untouched");
}

#[test]
fn outgrowing_the_buffer_refuses_and_resets() {
    let mut a = Assembly::new(8);
    a.begin(Opcode::Text, b"12345").unwrap();
    assert_eq!(a.append(b"6789"), Err(WsError::MessageTooLong { limit: 8 }));
    assert!(!a.is_open(), "nothing to resume after an overflow");
    assert_eq!(a.capacity(), 8);
}

#[test]
fn exactly_full_is_fine() {
    let mut a = Assembly::new(8);
    a.begin(Opcode::Binary, b"1234").unwrap();
    a.append(b"5678").unwrap();
    assert_eq!(a.take().unwrap().1, b"12345678");
}

#[test]
fn the_next_message_starts_from_zero() {
    let mut a = Assembly::new(16);
    a.begin(Opcode::Text, b"first").unwrap();
    a.take();
    a.begin(Opcode::Text, b"second").unwrap();
    assert_eq!(a.take().unwrap().1, b"second");
}

#[test]
fn reset_forgets_an_open_message() {
    let mut a = Assembly::new(16);
    a.begin(Opcode::Text, b"half").unwrap();
    a.reset();
    assert!(!a.is_open());
    assert!(a.bytes().is_empty());
}
