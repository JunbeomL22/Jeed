//! Fragments back into one message.
//!
//! ## The one copy on the receive path
//!
//! A server may split a message across frames — a text frame with `FIN`
//! clear, continuations, a final continuation with `FIN` set — and control
//! frames may land between them. The decoders walk one contiguous slice, so
//! fragments are copied here and the assembled message is handed over once.
//! A message that arrives in a single frame never touches this buffer; it is
//! decoded in place.
//!
//! ## Why oversize is an error and not a truncation
//!
//! A message longer than the buffer cannot be decoded at all: JSON cut short
//! is not a shallower book, it is not JSON. So the frame is refused,
//! [`WsError::MessageTooLong`] goes up, and the receive loop reconnects —
//! the same shape as a delta that will not fit the wire record
//! (`CLAUDE.md`).

use super::{Opcode, WsError};

/// The message under reassembly, if any.
#[derive(Debug)]
pub struct Assembly {
    buf: Box<[u8]>,
    len: usize,
    open: Option<Opcode>,
}

impl Assembly {
    /// A buffer holding up to `capacity` bytes of one message.
    ///
    /// Allocated once, here — the reassembly buffer outlives every
    /// connection the loop makes.
    pub fn new(capacity: usize) -> Self {
        Self { buf: vec![0; capacity].into_boxed_slice(), len: 0, open: None }
    }

    /// `true` while a fragmented message is waiting for its final frame.
    #[inline]
    pub const fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The kind of message being assembled.
    #[inline]
    pub const fn opcode(&self) -> Option<Opcode> {
        self.open
    }

    /// What has been gathered so far.
    #[inline]
    pub fn bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    /// Bytes the buffer holds.
    #[inline]
    pub const fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// Starts a message with its first fragment.
    ///
    /// [`WsError::InterleavedMessage`] if one is already open: data frames of
    /// two messages may not interleave (§5.4).
    pub fn begin(&mut self, opcode: Opcode, fragment: &[u8]) -> Result<(), WsError> {
        if self.open.is_some() {
            return Err(WsError::InterleavedMessage);
        }
        self.len = 0;
        self.open = Some(opcode);
        self.push(fragment)
    }

    /// Adds a continuation fragment.
    ///
    /// [`WsError::UnexpectedContinuation`] if no message is open.
    pub fn append(&mut self, fragment: &[u8]) -> Result<(), WsError> {
        if self.open.is_none() {
            return Err(WsError::UnexpectedContinuation);
        }
        self.push(fragment)
    }

    /// Closes the message and hands it over.
    ///
    /// The bytes stay valid until the next [`begin`](Self::begin).
    pub fn take(&mut self) -> Option<(Opcode, &[u8])> {
        let opcode = self.open.take()?;
        Some((opcode, &self.buf[..self.len]))
    }

    /// Forgets whatever was open — after a protocol error, or a reconnect.
    #[inline]
    pub fn reset(&mut self) {
        self.len = 0;
        self.open = None;
    }

    fn push(&mut self, fragment: &[u8]) -> Result<(), WsError> {
        let end = self.len + fragment.len();
        if end > self.buf.len() {
            self.reset();
            return Err(WsError::MessageTooLong { limit: self.buf.len() });
        }
        self.buf[self.len..end].copy_from_slice(fragment);
        self.len = end;
        Ok(())
    }
}
