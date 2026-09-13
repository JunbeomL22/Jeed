//! Single-pass JSON scanning, ported from `fractal-engine`'s
//! `data::exchanges::binance::json`.
//!
//! ## Why there is no `serde_json` here
//!
//! A `serde` decode of a depth frame allocates a `Vec` per side and a `String`
//! per number, on the receive path, per message. This walks the bytes once and
//! writes straight into the caller's [`WireRecord`] buffer — which for the
//! live path is a ring slot in shared memory, so a book update is parsed
//! directly into the segment with nothing staged in between.
//!
//! ## Two ways to read a key, and when each is allowed
//!
//! [`next_key`] returns the key's **first byte**. Binance's field names are one
//! or two characters and no two keys in a Binance message share a first byte —
//! `e` and `E`, `u` and `U`, `b` and `B` are all distinct because the match is
//! case-sensitive, and the two-character `pu` is matched as `p`, which collides
//! with nothing in a depth frame.
//!
//! That is a property of Binance's messages, not of JSON, and the other venues
//! do not have it. Upbit's `ask_price` and `ask_size` both begin `a`; Bybit's
//! envelope carries `topic`, `type` and `ts`; OKX's carries `action` beside
//! `arg`. Those decoders use [`next_field`], which returns the whole key and
//! costs a slice comparison instead of a byte comparison.
//!
//! Neither is a general JSON reader, and the rule for picking is not taste:
//! **use [`next_key`] only where no two keys in the message share a first
//! byte**, [`next_field`] everywhere else. A venue that adds a colliding key
//! later turns a first-byte decoder silently wrong, so a message anywhere near
//! that edge should already be read whole-key.
//!
//! ## Nested messages are read as subslices
//!
//! Neither scanner tracks object depth — both walk forward and rely on each
//! branch stepping over its own value, which keeps them at one level for as
//! long as the object is flat. Binance's frames are. The others wrap their
//! payload in an envelope (`{"topic":…,"data":{…}}`), so those decoders take
//! the envelope apart with [`object_at`] / [`objects_at`] and walk the inner
//! bytes as a message of their own. A subslice cannot run off the end of its
//! object, which is exactly the property a flat walk needs.
//!
//! ## What it does not do
//!
//! Unescape. [`parse_string_bytes`] returns the raw slice between the quotes
//! and steps over `\x` pairs so a quote inside a string does not end it early.
//! Every field these decoders read — a symbol, a decimal number — is
//! escape-free by construction, and a symbol that somehow arrived escaped
//! would fail [`Instrument::check_symbol`] rather than decode wrongly.
//!
//! [`WireRecord`]: jeed_wire::WireRecord
//! [`Instrument::check_symbol`]: crate::Instrument::check_symbol

use crate::error::CryptoError;
use crate::instrument::Instrument;
use jeed_wire::{WIRE_MAX_DEPTH, WireDeltaLevel, WireLevel};

// ============================================================================
// Scanning primitives
// ============================================================================

/// Finds the next key and returns `(first byte of the key, position of its
/// value)`. A key byte of `0` means the object is exhausted.
#[inline]
pub fn next_key(data: &[u8], mut pos: usize) -> (u8, usize) {
    while pos < data.len() {
        if data[pos] == b'"' {
            pos += 1;
            if pos < data.len() {
                let key = data[pos];
                pos += 1;
                while pos < data.len() && data[pos] != b':' {
                    pos += 1;
                }
                if pos < data.len() {
                    pos += 1;
                }
                return (key, skip_ws(data, pos));
            }
        } else {
            pos += 1;
        }
    }
    (0, pos)
}

/// Finds the next key and returns `(the key's bytes, position of its value)`.
///
/// An empty key means the object is exhausted. Unlike [`next_key`] this tells
/// apart keys that share a first byte, which every venue but Binance needs.
#[inline]
pub fn next_field(data: &[u8], mut pos: usize) -> (&[u8], usize) {
    while pos < data.len() && data[pos] != b'"' {
        pos += 1;
    }
    if pos >= data.len() {
        return (&[], pos);
    }
    let (key, mut pos) = parse_string_bytes(data, pos);
    pos = skip_ws(data, pos);
    if pos < data.len() && data[pos] == b':' {
        pos += 1;
    }
    (key, skip_ws(data, pos))
}

/// Steps over insignificant whitespace.
#[inline]
pub fn skip_ws(data: &[u8], mut pos: usize) -> usize {
    while pos < data.len() {
        match data[pos] {
            b' ' | b'\t' | b'\n' | b'\r' => pos += 1,
            _ => break,
        }
    }
    pos
}

/// Reads an unsigned integer, stepping over anything before the first digit.
#[inline]
pub fn parse_u64(data: &[u8], mut pos: usize) -> (u64, usize) {
    while pos < data.len() && !data[pos].is_ascii_digit() {
        pos += 1;
    }
    let mut out: u64 = 0;
    while pos < data.len() {
        let b = data[pos];
        if !b.is_ascii_digit() {
            break;
        }
        out = out.saturating_mul(10).saturating_add((b - b'0') as u64);
        pos += 1;
    }
    (out, pos)
}

/// Borrows a string value's bytes, without unescaping.
#[inline]
pub fn parse_string_bytes(data: &[u8], mut pos: usize) -> (&[u8], usize) {
    while pos < data.len() && data[pos] != b'"' {
        pos += 1;
    }
    if pos >= data.len() {
        return (&[], pos);
    }
    pos += 1;
    let start = pos;

    while pos < data.len() && data[pos] != b'"' {
        if data[pos] == b'\\' && pos + 1 < data.len() {
            pos += 2;
        } else {
            pos += 1;
        }
    }

    let bytes = &data[start..pos];
    if pos < data.len() {
        pos += 1;
    }
    (bytes, pos)
}

/// Reads `true` / `false`.
#[inline]
pub fn parse_bool(data: &[u8], mut pos: usize) -> (bool, usize) {
    pos = skip_ws(data, pos);
    if pos < data.len() && data[pos] == b't' {
        (true, (pos + 4).min(data.len()))
    } else {
        (false, (pos + 5).min(data.len()))
    }
}

/// Steps over one value of any type.
#[inline]
pub fn skip_value(data: &[u8], mut pos: usize) -> usize {
    pos = skip_ws(data, pos);
    if pos >= data.len() {
        return pos;
    }
    match data[pos] {
        b'{' => skip_bracketed(data, pos, b'{', b'}'),
        b'[' => skip_bracketed(data, pos, b'[', b']'),
        b'"' => parse_string_bytes(data, pos).1,
        b't' | b'f' | b'n' => skip_literal(data, pos),
        _ => skip_number(data, pos),
    }
}

/// Steps over a balanced `{…}` or `[…]`, `pos` on the opener.
///
/// One function for both, rather than `fractal-engine`'s `skip_object` and
/// `skip_array`: the two were byte-identical apart from the pair they counted,
/// and a nested `[` inside an object has to be counted anyway, so the string
/// skipping that makes it correct is the whole body.
#[inline]
fn skip_bracketed(data: &[u8], pos: usize, open: u8, close: u8) -> usize {
    skip_to_close(data, pos + 1, 1, open, close)
}

/// Steps to just past the `close` that balances `depth` already-open brackets.
fn skip_to_close(data: &[u8], mut pos: usize, mut depth: i32, open: u8, close: u8) -> usize {
    while pos < data.len() && depth > 0 {
        let b = data[pos];
        if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return pos + 1;
            }
        } else if b == b'"' {
            // A bracket inside a string is not a bracket.
            pos = parse_string_bytes(data, pos).1;
            continue;
        }
        pos += 1;
    }
    pos
}

/// Steps over `true` / `false` / `null`.
#[inline]
fn skip_literal(data: &[u8], mut pos: usize) -> usize {
    while pos < data.len() && data[pos].is_ascii_alphabetic() {
        pos += 1;
    }
    pos
}

/// Steps over a number.
#[inline]
fn skip_number(data: &[u8], mut pos: usize) -> usize {
    while pos < data.len() {
        match data[pos] {
            b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E' => pos += 1,
            _ => break,
        }
    }
    pos
}

/// Borrows a scalar value's bytes whether or not it is quoted.
///
/// Binance, OKX and Bybit send every price and size as a JSON **string**;
/// Upbit sends them as JSON **numbers**. Both reach a
/// [`DynamicExtractor`](jeed_convert::DynamicExtractor) as the same digits, so
/// the difference is one branch here rather than a second set of decoders.
///
/// `null` yields an empty slice, which the number readers reject as
/// [`ParseErr::Empty`](jeed_convert::ParseErr::Empty) — the right answer for a
/// field a decoder insisted on.
#[inline]
pub fn parse_scalar_bytes(data: &[u8], pos: usize) -> (&[u8], usize) {
    let pos = skip_ws(data, pos);
    if pos >= data.len() {
        return (&[], pos);
    }
    if data[pos] == b'"' {
        return parse_string_bytes(data, pos);
    }
    if data[pos] == b'n' {
        return (&[], skip_literal(data, pos));
    }
    let end = skip_number(data, pos);
    (&data[pos..end], end)
}

/// Reads an unsigned integer whether or not it is quoted.
///
/// Necessary and not merely convenient: [`parse_u64`] stops on the first
/// non-digit, which for `"ts":"1706000000000"` is the value's **closing**
/// quote — and the next scan would then read that quote as the opening one of
/// a key. OKX quotes its timestamps and leaves its sequence numbers bare, in
/// the same object.
#[inline]
pub fn parse_scalar_u64(data: &[u8], pos: usize) -> (u64, usize) {
    let (bytes, next) = parse_scalar_bytes(data, pos);
    let mut out: u64 = 0;
    for &b in bytes {
        if !b.is_ascii_digit() {
            break;
        }
        out = out.saturating_mul(10).saturating_add((b - b'0') as u64);
    }
    (out, next)
}

// ============================================================================
// Envelopes
// ============================================================================

/// Borrows the bytes **between the braces** of the object at `pos`, and gives
/// the position past its `}`.
///
/// `None` when there is no object here, or it never closes. The inner slice is
/// what the flat scanners want: walking it cannot wander into the enclosing
/// object, because there is nothing after the object's last field.
#[inline]
pub fn object_at(data: &[u8], pos: usize) -> Option<(&[u8], usize)> {
    let open = skip_ws(data, pos);
    if open >= data.len() || data[open] != b'{' {
        return None;
    }
    let past = skip_bracketed(data, open, b'{', b'}');
    if past <= open + 1 || data[past - 1] != b'}' {
        return None;
    }
    Some((&data[open + 1..past - 1], past))
}

/// The objects of a `[{…},{…}]` array, one inner slice at a time.
///
/// OKX wraps everything in `"data":[…]` and puts a whole batch of prints in
/// it; Bybit does the same under `publicTrade`. Iterating rather than taking
/// the last object is the difference between publishing every print and
/// publishing one of them.
#[derive(Debug, Clone)]
pub struct Objects<'a> {
    /// The whole payload; positions below index into it.
    data: &'a [u8],

    /// Just past the last object handed out.
    pos: usize,
}

/// Positions an [`Objects`] at the first element of the array at `pos`.
///
/// `None` when there is no array here — required to be *here* and not
/// somewhere ahead, for the reason [`quote_levels`] gives.
#[inline]
pub fn objects_at(data: &[u8], pos: usize) -> Option<Objects<'_>> {
    let open = skip_ws(data, pos);
    if open >= data.len() || data[open] != b'[' {
        return None;
    }
    Some(Objects { data, pos: open + 1 })
}

impl<'a> Iterator for Objects<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        loop {
            self.pos = skip_ws(self.data, self.pos);
            if self.pos >= self.data.len() || self.data[self.pos] == b']' {
                return None;
            }
            if self.data[self.pos] == b',' {
                self.pos += 1;
                continue;
            }
            let (inner, past) = object_at(self.data, self.pos)?;
            self.pos = past;
            return Some(inner);
        }
    }
}

// ============================================================================
// Level arrays
// ============================================================================

/// Steps to just past the `]` closing an outer array we are already inside.
#[inline]
fn skip_rest_of_array(data: &[u8], pos: usize) -> usize {
    skip_to_close(data, pos, 1, b'[', b']')
}

/// Positions at the first element of `[["p","q"],…]`, or returns `None` when
/// there is no array here.
#[inline]
fn enter_array(data: &[u8], pos: usize) -> Option<usize> {
    let pos = skip_ws(data, pos);
    // Required to be *here*, not somewhere ahead: scanning forward for the
    // next `[` would silently adopt a later field's array as this one's.
    if pos < data.len() && data[pos] == b'[' { Some(pos + 1) } else { None }
}

/// Reads `[["price","qty"], …]` into a snapshot side, capped at
/// [`WIRE_MAX_DEPTH`].
///
/// Returns `(depth, position past the array)`, where `depth` is the number of
/// levels up to and including the deepest one carrying quantity — counted, not
/// assumed, the same rule `jeed_krx::decode::common::BookAccum` applies
/// (`CLAUDE.md`).
///
/// Levels past [`WIRE_MAX_DEPTH`] are **skipped, not parsed**. A REST snapshot
/// can be five thousand levels deep and the wire carries ten; walking the
/// other 4,990 through the decimal parser would be the most expensive thing
/// this crate does, for a result that is discarded.
pub fn quote_levels(
    inst: &Instrument,
    data: &[u8],
    pos: usize,
    out: &mut [WireLevel; WIRE_MAX_DEPTH],
) -> Result<(u8, usize), CryptoError> {
    let Some(mut pos) = enter_array(data, pos) else {
        return Ok((0, data.len()));
    };

    let mut written = 0usize;
    let mut depth = 0usize;

    loop {
        pos = skip_ws(data, pos);
        if pos >= data.len() {
            break;
        }
        match data[pos] {
            b']' => {
                pos += 1;
                break;
            }
            b',' => pos += 1,
            b'[' => {
                let (price, qty, next) = pair(inst, data, pos)?;
                pos = next;
                out[written] = WireLevel::new(price, qty);
                written += 1;
                if qty > 0 {
                    depth = written;
                }
                if written == WIRE_MAX_DEPTH {
                    pos = skip_rest_of_array(data, pos);
                    break;
                }
            }
            _ => pos += 1,
        }
    }

    Ok((depth as u8, pos))
}

/// Reads `[["price","qty"], …]` into a delta side.
///
/// Returns `(count, position past the array)`. Unlike [`quote_levels`] this
/// **refuses** rather than truncates when `out` runs out: a snapshot that
/// keeps ten of five thousand levels is a shallower book, while a delta that
/// keeps thirty of forty changes is a wrong book, permanently
/// ([`SnapshotDeltaPayload`](jeed_wire::SnapshotDeltaPayload)).
pub fn delta_levels(
    inst: &Instrument,
    data: &[u8],
    pos: usize,
    out: &mut [WireDeltaLevel],
) -> Result<(usize, usize), CryptoError> {
    let Some(mut pos) = enter_array(data, pos) else {
        return Ok((0, data.len()));
    };

    let mut written = 0usize;

    loop {
        pos = skip_ws(data, pos);
        if pos >= data.len() {
            break;
        }
        match data[pos] {
            b']' => {
                pos += 1;
                break;
            }
            b',' => pos += 1,
            b'[' => {
                if written == out.len() {
                    return Err(CryptoError::DeltaOverflow {
                        max: jeed_wire::WIRE_MAX_DELTA_LEVELS,
                    });
                }
                let (price, qty, next) = pair(inst, data, pos)?;
                pos = next;
                out[written] = WireDeltaLevel { price, qty };
                written += 1;
            }
            _ => pos += 1,
        }
    }

    Ok((written, pos))
}

/// Steps over one `,` if that is what comes next.
///
/// A quoted element is self-delimiting, so the comma between two of them can
/// simply be scanned past. A bare number is not: in `[152430000.0,0.23]` the
/// comma is where the first number ends, and without stepping over it the
/// second read would start on it and find no digits.
#[inline]
fn skip_comma(data: &[u8], pos: usize) -> usize {
    let pos = skip_ws(data, pos);
    if pos < data.len() && data[pos] == b',' { pos + 1 } else { pos }
}

/// Reads one `["price","qty"]` or `[price,qty]`, `pos` on the opening `[`.
#[inline]
fn pair(inst: &Instrument, data: &[u8], pos: usize) -> Result<(i64, u64, usize), CryptoError> {
    let (price_bytes, pos) = parse_scalar_bytes(data, pos + 1);
    let price = inst.price(price_bytes, "level price")?;

    let (qty_bytes, mut pos) = parse_scalar_bytes(data, skip_comma(data, pos));
    let qty = inst.qty(qty_bytes, "level quantity")?;

    // Past whatever else the inner array holds. Binance sends a third,
    // always-empty element on some endpoints and has since dropped it; a
    // trailing element must not be mistaken for the next level.
    pos = skip_ws(data, pos);
    if pos < data.len() && data[pos] != b']' {
        pos = skip_rest_of_array(data, pos);
    } else if pos < data.len() {
        pos += 1;
    }

    Ok((price, qty, pos))
}
