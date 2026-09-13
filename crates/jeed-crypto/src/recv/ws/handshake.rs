//! The opening handshake: one HTTP request, one `101`, then frames.
//!
//! ```text
//! GET /ws/btcusdt@trade HTTP/1.1
//! Host: stream.binance.com:9443
//! Upgrade: websocket
//! Connection: Upgrade
//! Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==
//! Sec-WebSocket-Version: 13
//!
//! HTTP/1.1 101 Switching Protocols
//! Upgrade: websocket
//! Connection: Upgrade
//! Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=
//! ```
//!
//! ## The accept value is checked
//!
//! `Sec-WebSocket-Accept` is `base64(sha1(key + GUID))`, and RFC 6455 §4.1
//! says a client **must** fail the connection if it is missing or wrong. It
//! is cheap to check and it is the difference between "a server that speaks
//! WebSocket answered" and "something answered 101". A load balancer that
//! upgrades blindly and then forwards plain HTTP would pass the second test
//! and fail on the first frame, with a much less useful error.
//!
//! ## What is not sent
//!
//! No `Origin` (that is for browsers), no `Sec-WebSocket-Protocol`, no
//! `Sec-WebSocket-Extensions`. Not offering an extension is what guarantees
//! the server cannot select one, and that is what keeps every server frame
//! plain bytes (`super`).

use super::WsError;
use super::sha1::sha1;
use crate::recv::endpoint::Endpoint;

/// Length of the base64 `Sec-WebSocket-Key` — sixteen random bytes encoded.
pub const KEY_LEN: usize = 24;

/// Length of the base64 `Sec-WebSocket-Accept` — a SHA-1 encoded.
pub const ACCEPT_LEN: usize = 28;

/// The GUID the protocol appends to the key before hashing (§1.3).
const GUID: &[u8; 36] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encodes sixteen bytes as the `Sec-WebSocket-Key` header value.
///
/// The bytes need not be secret, only fresh: the key exists so a cached
/// response from an intermediary cannot pass as a live handshake.
pub fn key(nonce: &[u8; 16]) -> [u8; KEY_LEN] {
    let mut out = [0u8; KEY_LEN];
    base64(nonce, &mut out);
    out
}

/// The `Sec-WebSocket-Accept` a compliant server answers `key` with.
pub fn accept(key: &[u8; KEY_LEN]) -> [u8; ACCEPT_LEN] {
    let mut joined = [0u8; KEY_LEN + GUID.len()];
    joined[..KEY_LEN].copy_from_slice(key);
    joined[KEY_LEN..].copy_from_slice(GUID);
    let mut out = [0u8; ACCEPT_LEN];
    base64(&sha1(&joined), &mut out);
    out
}

/// Standard base64 with padding. `out` must be `4 * ceil(len / 3)` long.
fn base64(input: &[u8], out: &mut [u8]) {
    let mut chunks = input.chunks_exact(3);
    let mut pos = 0;
    for c in &mut chunks {
        let n = (u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2]);
        out[pos] = ALPHABET[(n >> 18) as usize & 63];
        out[pos + 1] = ALPHABET[(n >> 12) as usize & 63];
        out[pos + 2] = ALPHABET[(n >> 6) as usize & 63];
        out[pos + 3] = ALPHABET[n as usize & 63];
        pos += 4;
    }
    match *chunks.remainder() {
        [a] => {
            let n = u32::from(a) << 16;
            out[pos] = ALPHABET[(n >> 18) as usize & 63];
            out[pos + 1] = ALPHABET[(n >> 12) as usize & 63];
            out[pos + 2] = b'=';
            out[pos + 3] = b'=';
        }
        [a, b] => {
            let n = (u32::from(a) << 16) | (u32::from(b) << 8);
            out[pos] = ALPHABET[(n >> 18) as usize & 63];
            out[pos + 1] = ALPHABET[(n >> 12) as usize & 63];
            out[pos + 2] = ALPHABET[(n >> 6) as usize & 63];
            out[pos + 3] = b'=';
        }
        _ => {}
    }
}

/// Writes the upgrade request for `endpoint` into `out`. Returns its length.
///
/// `Host` carries the port only when it is not the scheme's default, which is
/// what the URL grammar means and what a virtual-hosting front end compares
/// against.
pub fn request(out: &mut [u8], endpoint: &Endpoint, key: &[u8; KEY_LEN]) -> Result<usize, WsError> {
    let mut w = Writer { out, pos: 0 };
    w.put(b"GET ")?;
    w.put(endpoint.path.as_bytes())?;
    w.put(b" HTTP/1.1\r\nHost: ")?;
    w.put(endpoint.host.as_bytes())?;
    if endpoint.port != endpoint.default_port() {
        w.put(b":")?;
        w.decimal(endpoint.port)?;
    }
    w.put(b"\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: ")?;
    w.put(key)?;
    w.put(b"\r\nSec-WebSocket-Version: 13\r\n\r\n")?;
    Ok(w.pos)
}

/// Checks the server's response at the start of `buf`.
///
/// `Ok(Some(n))` means the response occupies the first `n` bytes and is a
/// correct upgrade; whatever follows is the first frame. `Ok(None)` means the
/// headers have not all arrived. `Err` means the connection must be failed.
pub fn parse_response(buf: &[u8], expected: &[u8; ACCEPT_LEN]) -> Result<Option<usize>, WsError> {
    let Some(end) = find(buf, b"\r\n\r\n") else {
        return Ok(None);
    };
    let head = &buf[..end];
    let mut lines = head.split(|&b| b == b'\n').map(|l| l.strip_suffix(b"\r").unwrap_or(l));

    let status = lines.next().ok_or(WsError::MalformedResponse)?;
    let code = status
        .strip_prefix(b"HTTP/1.1 ")
        .and_then(|s| s.get(..3))
        .and_then(parse_u16)
        .ok_or(WsError::MalformedResponse)?;
    if code != 101 {
        return Err(WsError::Status(code));
    }

    let (mut upgrade, mut connection, mut accepted) = (false, false, false);
    for line in lines {
        let Some(colon) = line.iter().position(|&b| b == b':') else {
            return Err(WsError::MalformedResponse);
        };
        let name = trim(&line[..colon]);
        let value = trim(&line[colon + 1..]);
        if name.eq_ignore_ascii_case(b"upgrade") {
            upgrade = value.eq_ignore_ascii_case(b"websocket");
        } else if name.eq_ignore_ascii_case(b"connection") {
            connection = value.split(|&b| b == b',').any(|t| trim(t).eq_ignore_ascii_case(b"upgrade"));
        } else if name.eq_ignore_ascii_case(b"sec-websocket-accept") {
            accepted = value == expected;
        } else if name.eq_ignore_ascii_case(b"sec-websocket-extensions") && !value.is_empty() {
            return Err(WsError::Extension);
        } else if name.eq_ignore_ascii_case(b"sec-websocket-protocol") && !value.is_empty() {
            return Err(WsError::Subprotocol);
        }
    }

    if !upgrade {
        return Err(WsError::Upgrade);
    }
    if !connection {
        return Err(WsError::Connection);
    }
    if !accepted {
        return Err(WsError::Accept);
    }
    Ok(Some(end + 4))
}

/// Appends into a fixed slice, refusing rather than truncating.
struct Writer<'a> {
    out: &'a mut [u8],
    pos: usize,
}

impl Writer<'_> {
    fn put(&mut self, bytes: &[u8]) -> Result<(), WsError> {
        let end = self.pos + bytes.len();
        if end > self.out.len() {
            return Err(WsError::NoRoom);
        }
        self.out[self.pos..end].copy_from_slice(bytes);
        self.pos = end;
        Ok(())
    }

    fn decimal(&mut self, mut n: u16) -> Result<(), WsError> {
        let mut digits = [0u8; 5];
        let mut i = digits.len();
        loop {
            i -= 1;
            digits[i] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        self.put(&digits[i..])
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn trim(bytes: &[u8]) -> &[u8] {
    let start = bytes.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(bytes.len());
    let end = bytes.iter().rposition(|b| !b.is_ascii_whitespace()).map_or(start, |i| i + 1);
    &bytes[start..end]
}

fn parse_u16(bytes: &[u8]) -> Option<u16> {
    let mut n: u16 = 0;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add(u16::from(b - b'0'))?;
    }
    Some(n)
}
