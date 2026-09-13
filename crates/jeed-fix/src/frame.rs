//! Message framing: `8=…<SOH>9=len<SOH>` … `10=ddd<SOH>`.
//!
//! `BodyLength` (9) counts every byte after its own `<SOH>` up to and
//! including the `<SOH>` that precedes `10=`. `CheckSum` (10) is the byte
//! sum of everything before `10=` modulo 256, as three decimal digits. This
//! is the FIX counterpart of the KRX "length + end marker" validation: both
//! are checked before a single body field is looked at.

use crate::error::FixError;
use crate::tagvalue::{parse_u64, Fields};
use crate::SOH;

/// Largest `BodyLength` accepted. A 10-deep two-sided snapshot with
/// per-entry timestamps is under 1 KiB; anything near this bound is a
/// desynchronised stream, not a message.
pub const FIX_MAX_BODY_LENGTH: usize = 1 << 16;

/// Length of the trailer `10=ddd<SOH>`.
pub const FIX_TRAILER_LEN: usize = 7;

/// One complete, checksum-verified message borrowed from a buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame<'a> {
    /// The whole message, `8=` through the trailer `<SOH>`.
    pub bytes: &'a [u8],

    /// Value of `8=` (e.g. `FIX.4.4`).
    pub begin_string: &'a [u8],

    /// Body: from the byte after `9=…<SOH>` up to (not including) `10=`.
    /// Ends with `<SOH>`.
    pub body: &'a [u8],
}

impl<'a> Frame<'a> {
    /// Total length of the message in bytes.
    #[inline]
    pub const fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Never true for a parsed frame; provided for clippy's `len` idiom.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Field iterator over the body.
    #[inline]
    pub const fn fields(&self) -> Fields<'a> {
        Fields::new(self.body)
    }
}

/// Byte sum modulo 256 — the `CheckSum` of `bytes`.
#[inline]
pub fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |acc, &b| acc.wrapping_add(b))
}

/// Frames the message at the head of `buf`.
///
/// Returns [`FixError::Incomplete`] when `buf` holds a valid prefix of a
/// message but not all of it; the caller reads more bytes and retries.
/// Every other error means the stream is out of sync at `buf[0]`.
pub fn frame(buf: &[u8]) -> Result<Frame<'_>, FixError> {
    // 8=<begin_string><SOH>
    if buf.is_empty() {
        return Err(FixError::Incomplete);
    }
    if buf.len() < 2 {
        return Err(if buf[0] == b'8' { FixError::Incomplete } else { FixError::BeginString });
    }
    if &buf[..2] != b"8=" {
        return Err(FixError::BeginString);
    }
    let Some(bs_end) = buf[2..].iter().position(|&b| b == SOH) else {
        return Err(if buf.len() > 2 + 16 { FixError::BeginString } else { FixError::Incomplete });
    };
    let begin_string = &buf[2..2 + bs_end];
    if begin_string.is_empty() {
        return Err(FixError::BeginString);
    }
    let len_start = 2 + bs_end + 1;

    // 9=<digits><SOH>
    let rest = &buf[len_start.min(buf.len())..];
    if rest.len() < 2 {
        return Err(FixError::Incomplete);
    }
    if &rest[..2] != b"9=" {
        return Err(FixError::BodyLength { declared: u32::MAX });
    }
    let Some(digits_end) = rest[2..].iter().position(|&b| b == SOH) else {
        return Err(if rest.len() > 2 + 6 {
            FixError::BodyLength { declared: u32::MAX }
        } else {
            FixError::Incomplete
        });
    };
    let digits = &rest[2..2 + digits_end];
    let Some(body_len) = parse_u64(digits) else {
        return Err(FixError::BodyLength { declared: u32::MAX });
    };
    if body_len == 0 || body_len as usize > FIX_MAX_BODY_LENGTH {
        return Err(FixError::BodyLength { declared: body_len.min(u32::MAX as u64) as u32 });
    }
    let body_len = body_len as usize;
    let body_start = len_start + 2 + digits_end + 1;
    let trailer_start = body_start + body_len;
    let total = trailer_start + FIX_TRAILER_LEN;
    if buf.len() < total {
        return Err(FixError::Incomplete);
    }

    // Body must end on a field boundary.
    if buf[trailer_start - 1] != SOH {
        return Err(FixError::BodyLength { declared: body_len as u32 });
    }

    // 10=ddd<SOH>
    let trailer = &buf[trailer_start..total];
    if &trailer[..3] != b"10=" || trailer[6] != SOH {
        return Err(FixError::Trailer);
    }
    let Some(found) = parse_u64(&trailer[3..6]) else {
        return Err(FixError::Trailer);
    };
    let expected = checksum(&buf[..trailer_start]);
    if found > 255 || found as u8 != expected {
        return Err(FixError::CheckSum { expected, found: found.min(255) as u8 });
    }

    Ok(Frame {
        bytes: &buf[..total],
        begin_string,
        body: &buf[body_start..trailer_start],
    })
}

/// Fixed-capacity receive buffer that turns a byte stream into frames.
///
/// ```text
/// loop {
///     let n = socket.read(buf.spare())?;   // append raw bytes
///     buf.commit(n);
///     while let Some(frame) = buf.peek()? { handle(frame); buf.consume(frame.len()); }
/// }
/// ```
///
/// `N` bounds one message: a message longer than the buffer can never
/// complete and surfaces as [`FixError::BodyLength`] via [`peek`](Self::peek).
#[derive(Debug, Clone)]
pub struct FrameBuffer<const N: usize> {
    buf: [u8; N],

    /// Bytes `[0, len)` are unread stream data.
    len: usize,
}

impl<const N: usize> Default for FrameBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> FrameBuffer<N> {
    /// Empty buffer.
    pub const fn new() -> Self {
        Self { buf: [0; N], len: 0 }
    }

    /// Unread bytes.
    #[inline]
    pub fn filled(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    /// Writable tail for the next `read`.
    #[inline]
    pub fn spare(&mut self) -> &mut [u8] {
        &mut self.buf[self.len..]
    }

    /// Marks `n` bytes of [`spare`](Self::spare) as filled.
    #[inline]
    pub fn commit(&mut self, n: usize) {
        self.len = (self.len + n).min(N);
    }

    /// Appends `bytes`; returns how many fit.
    #[inline]
    pub fn push(&mut self, bytes: &[u8]) -> usize {
        let n = bytes.len().min(N - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&bytes[..n]);
        self.len += n;
        n
    }

    /// Frames the message at the head without consuming it. `Ok(None)`
    /// means more bytes are needed; a full buffer that still does not
    /// frame is reported as an oversize body.
    pub fn peek(&self) -> Result<Option<Frame<'_>>, FixError> {
        match frame(self.filled()) {
            Ok(f) => Ok(Some(f)),
            Err(FixError::Incomplete) if self.len == N => {
                Err(FixError::BodyLength { declared: N as u32 })
            }
            Err(FixError::Incomplete) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Drops the first `n` bytes.
    #[inline]
    pub fn consume(&mut self, n: usize) {
        let n = n.min(self.len);
        self.buf.copy_within(n..self.len, 0);
        self.len -= n;
    }

    /// Discards everything (resynchronise after a framing error).
    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
    }
}
