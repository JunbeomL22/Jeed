//! Frame headers in, masked frames out.
//!
//! ```text
//!  0               1               2               3
//!  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
//! +-+-+-+-+-------+-+-------------+-------------------------------+
//! |F|R|R|R| opcode|M| Payload len |    Extended payload length    |
//! |I|S|S|S|  (4)  |A|     (7)     |             (16/64)           |
//! |N|V|V|V|       |S|             |   (if payload len==126/127)   |
//! +-+-+-+-+-------+-+-------------+-------------------------------+
//! |                    Masking-key, if MASK set                   |
//! +-------------------------------+-------------------------------+
//! |                          Payload Data                         |
//! ```
//!
//! Reading and writing are asymmetric on purpose. [`parse_header`] refuses a
//! mask bit because a server must never set one; [`write_frame`] always sets
//! one because a client must always. A parser that also unmasked would be
//! code for a peer that does not exist.

use super::WsError;

/// Longest control-frame payload the protocol allows (§5.5).
pub const MAX_CONTROL_PAYLOAD: usize = 125;

/// Longest frame header: two bytes, an eight-byte length, a four-byte mask.
pub const MAX_HEADER_LEN: usize = 14;

/// What a frame is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Opcode {
    /// The rest of a fragmented message.
    Continuation = 0x0,
    /// UTF-8 text. Every venue in this crate sends its JSON as this.
    Text = 0x1,
    /// Bytes. No venue in this crate sends one; a compressed feed would.
    Binary = 0x2,
    /// The peer is hanging up, and says why in the first two bytes.
    Close = 0x8,
    /// Answer with a [`Pong`](Self::Pong) carrying the same payload.
    Ping = 0x9,
    /// The answer to a ping — ours, or one the peer sent unsolicited.
    Pong = 0xA,
}

impl Opcode {
    /// The opcode for a nibble, or `None` for the reserved ones.
    #[inline]
    pub const fn from_u8(b: u8) -> Option<Self> {
        Some(match b {
            0x0 => Self::Continuation,
            0x1 => Self::Text,
            0x2 => Self::Binary,
            0x8 => Self::Close,
            0x9 => Self::Ping,
            0xA => Self::Pong,
            _ => return None,
        })
    }

    /// The nibble.
    #[inline]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// `true` for close, ping and pong — the frames that may arrive in the
    /// middle of a fragmented message and are never fragmented themselves.
    #[inline]
    pub const fn is_control(self) -> bool {
        self.as_u8() & 0x8 != 0
    }
}

/// A server frame's header, decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// `true` when this frame ends its message.
    pub fin: bool,

    /// What the frame is.
    pub opcode: Opcode,

    /// Header bytes — 2, 4 or 10.
    pub header_len: usize,

    /// Payload bytes that follow the header.
    pub payload_len: usize,
}

impl FrameHeader {
    /// Header plus payload.
    #[inline]
    pub const fn len(&self) -> usize {
        self.header_len + self.payload_len
    }

    /// `true` for a frame with no payload — a bare ping, an empty close.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.payload_len == 0
    }
}

/// Decodes the header at the start of `buf`.
///
/// `Ok(None)` means the header is not all there yet. `Err` means the peer
/// broke the protocol, and the connection should go with it.
pub fn parse_header(buf: &[u8]) -> Result<Option<FrameHeader>, WsError> {
    let [b0, b1, rest @ ..] = buf else {
        return Ok(None);
    };

    if b0 & 0x70 != 0 {
        return Err(WsError::ReservedBits);
    }
    let opcode = Opcode::from_u8(b0 & 0x0F).ok_or(WsError::Opcode(b0 & 0x0F))?;
    let fin = b0 & 0x80 != 0;
    if b1 & 0x80 != 0 {
        return Err(WsError::MaskedByServer);
    }

    let (header_len, payload_len) = match b1 & 0x7F {
        126 => {
            let [hi, lo, ..] = rest else {
                return Ok(None);
            };
            (4, u16::from_be_bytes([*hi, *lo]) as usize)
        }
        127 => {
            let Some(bytes) = rest.get(..8) else {
                return Ok(None);
            };
            let n = u64::from_be_bytes(bytes.try_into().expect("8 bytes"));
            if n >> 63 != 0 {
                return Err(WsError::Length);
            }
            let n = usize::try_from(n).map_err(|_| WsError::Length)?;
            (10, n)
        }
        n => (2, n as usize),
    };

    if opcode.is_control() {
        if !fin {
            return Err(WsError::FragmentedControl);
        }
        if payload_len > MAX_CONTROL_PAYLOAD {
            return Err(WsError::ControlTooLong(payload_len));
        }
    }

    Ok(Some(FrameHeader { fin, opcode, header_len, payload_len }))
}

/// Bytes a masked client frame of `payload_len` occupies.
#[inline]
pub const fn framed_len(payload_len: usize) -> usize {
    let ext = if payload_len < 126 {
        0
    } else if payload_len <= u16::MAX as usize {
        2
    } else {
        8
    };
    2 + ext + 4 + payload_len
}

/// Writes one complete, masked, `FIN` frame into `out`. Returns its length.
///
/// The payload is XOR-ed with `mask` as it is copied, so this is the one
/// pass over the bytes. `mask` need not be unpredictable here: masking exists
/// to defeat cache-poisoning of intermediaries that see plaintext, and every
/// venue connection is TLS.
pub fn write_frame(
    out: &mut [u8],
    opcode: Opcode,
    payload: &[u8],
    mask: [u8; 4],
) -> Result<usize, WsError> {
    if opcode.is_control() && payload.len() > MAX_CONTROL_PAYLOAD {
        return Err(WsError::ControlTooLong(payload.len()));
    }
    let total = framed_len(payload.len());
    if out.len() < total {
        return Err(WsError::NoRoom);
    }

    out[0] = 0x80 | opcode.as_u8();
    let mut pos = 2;
    match payload.len() {
        n if n < 126 => out[1] = 0x80 | n as u8,
        n if n <= u16::MAX as usize => {
            out[1] = 0x80 | 126;
            out[2..4].copy_from_slice(&(n as u16).to_be_bytes());
            pos = 4;
        }
        n => {
            out[1] = 0x80 | 127;
            out[2..10].copy_from_slice(&(n as u64).to_be_bytes());
            pos = 10;
        }
    }
    out[pos..pos + 4].copy_from_slice(&mask);
    pos += 4;

    for (i, (dst, src)) in out[pos..total].iter_mut().zip(payload).enumerate() {
        *dst = src ^ mask[i & 3];
    }
    Ok(total)
}

/// Writes a masked Close frame carrying `code` and no reason.
///
/// A reason is for humans reading the other side's log, and the other side
/// is a venue that will not read it.
pub fn write_close(out: &mut [u8], code: u16, mask: [u8; 4]) -> Result<usize, WsError> {
    write_frame(out, Opcode::Close, &code.to_be_bytes(), mask)
}

/// The status code in a Close frame's payload, if it carried one.
///
/// An empty Close is legal and means "no reason given"; one byte is not
/// (§5.5.1) and reads as no code rather than as an error — the peer is
/// hanging up either way.
#[inline]
pub fn close_code(payload: &[u8]) -> Option<u16> {
    match payload {
        [hi, lo, ..] => Some(u16::from_be_bytes([*hi, *lo])),
        _ => None,
    }
}
