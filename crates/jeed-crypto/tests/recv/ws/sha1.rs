//! Tests for `src/recv/ws/sha1.rs` — the standard vectors, and the two
//! padding boundaries.

use jeed_crypto::recv::ws::sha1::sha1;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn the_empty_message() {
    assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
}

#[test]
fn abc() {
    assert_eq!(hex(&sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
}

#[test]
fn fifty_six_bytes_needs_a_second_block_for_the_length() {
    // The FIPS 180 two-block example: exactly 56 bytes leaves no room for
    // the eight length bytes after the 0x80, so padding spills into a
    // second block.
    let msg = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
    assert_eq!(msg.len(), 56);
    assert_eq!(hex(&sha1(msg)), "84983e441c3bd26ebaae4aa1f95129e5e54670f1");
}

#[test]
fn fifty_five_bytes_fits_in_one_block() {
    let msg = [b'a'; 55];
    assert_eq!(hex(&sha1(&msg)), "c1c8bbdc22796e28c0e15163d20899b65621d65a");
}

#[test]
fn exactly_one_block_of_input() {
    let msg = [b'a'; 64];
    assert_eq!(hex(&sha1(&msg)), "0098ba824b5c16427bd7a1122a5a442a25ec644d");
}

#[test]
fn a_million_a() {
    let msg = vec![b'a'; 1_000_000];
    assert_eq!(hex(&sha1(&msg)), "34aa973cd4c4daa4f61eeb2bdbad27316534016f");
}
