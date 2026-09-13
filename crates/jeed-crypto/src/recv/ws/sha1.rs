//! SHA-1, for the one thing RFC 6455 still uses it for.
//!
//! The opening handshake proves the server speaks WebSocket by echoing our key
//! hashed together with a fixed GUID (`Sec-WebSocket-Accept`). That is the
//! whole use: sixty bytes, once per connection, with no secrecy resting on it
//! — the transport's integrity is TLS's job. rustls's provider hashes for TLS
//! (SHA-256 and up) and does not export SHA-1, so a hashing crate would be a
//! dependency for one call on the cold path. Sixty lines are cheaper, and
//! the test vectors are in the RFC.

/// SHA-1 of `data`.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];

    let mut blocks = data.chunks_exact(64);
    for block in &mut blocks {
        compress(&mut h, block.try_into().expect("64-byte chunk"));
    }

    // Padding: a 1 bit, zeros, and the length in bits — in one block if the
    // tail leaves room for the eight length bytes, else in two.
    let rem = blocks.remainder();
    let mut tail = [0u8; 128];
    tail[..rem.len()].copy_from_slice(rem);
    tail[rem.len()] = 0x80;
    let total = if rem.len() < 56 { 64 } else { 128 };
    let bits = (data.len() as u64).wrapping_mul(8);
    tail[total - 8..total].copy_from_slice(&bits.to_be_bytes());
    for block in tail[..total].chunks_exact(64) {
        compress(&mut h, block.try_into().expect("64-byte chunk"));
    }

    let mut out = [0u8; 20];
    for (dst, word) in out.chunks_exact_mut(4).zip(h) {
        dst.copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// One block of the compression function.
fn compress(h: &mut [u32; 5], block: &[u8; 64]) {
    let mut w = [0u32; 80];
    for (dst, src) in w.iter_mut().zip(block.chunks_exact(4)) {
        *dst = u32::from_be_bytes(src.try_into().expect("4-byte chunk"));
    }
    for i in 16..80 {
        w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
    }

    let [mut a, mut b, mut c, mut d, mut e] = *h;
    for (i, &wi) in w.iter().enumerate() {
        let (f, k) = match i {
            0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
            20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
            40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
            _ => (b ^ c ^ d, 0xCA62_C1D6),
        };
        let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(wi);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = t;
    }

    h[0] = h[0].wrapping_add(a);
    h[1] = h[1].wrapping_add(b);
    h[2] = h[2].wrapping_add(c);
    h[3] = h[3].wrapping_add(d);
    h[4] = h[4].wrapping_add(e);
}
