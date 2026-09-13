//! Outbound session messages — the half `jeed-krx` never needs.
//!
//! A multicast handler that sends nothing still receives everything. A FIX
//! handler that sends nothing receives **nothing at all**: the session starts
//! with our Logon, stays open only because we answer silence, and delivers data
//! only because we asked for it (`documents/feed_handler.md` §10).
//!
//! ```text
//! connect ──→ 35=A Logon ──→ 35=V MarketDataRequest ──→ [W/X flow]
//!                 ↑                                        │
//!                 │  35=0 Heartbeat   ←── our silence ─────┤
//!                 │  35=1 TestRequest ←── their silence ───┤
//!                 │  35=2 ResendRequest ←── a 34 gap ──────┤
//!                 └─ 35=5 Logout      ←── a 34 regression ─┘
//! ```
//!
//! ## Nothing here allocates or reads a clock
//!
//! Every builder writes into a caller-supplied buffer and takes `now` as an
//! argument. The receive loop is a pinned busy-spin (`CLAUDE.md`), and an
//! encoder that allocated per heartbeat would put a `malloc` on the same thread
//! as the book.
//!
//! ## What is deliberately *not* here
//!
//! Orders. This is the market-data session; `documents/feed_handler.md` §13
//! leaves the order session to quickfix-rs, and an encoder that could spell
//! `35=D` would be an invitation to route one through the feed handler.

use crate::frame::checksum;
use crate::market_data::{FixText, MdEntryType};
use crate::{FIX_BEGIN_STRING, SOH};
use core::fmt;
use jeed_wire::UnixNano;

/// Largest message this module emits, and the buffer size every builder wants.
///
/// Sized by [`SubscriptionRequest`], the only one that grows with
/// configuration. The admin messages are all under 128 bytes.
pub const MAX_EMIT_LEN: usize = 4096;

/// Nanoseconds in a second.
const NS_PER_SEC: u64 = 1_000_000_000;

/// Session tags this module writes.
mod tags {
    pub const MSG_TYPE: u32 = 35;
    pub const SENDER_COMP_ID: u32 = 49;
    pub const TARGET_COMP_ID: u32 = 56;
    pub const MSG_SEQ_NUM: u32 = 34;
    pub const SENDING_TIME: u32 = 52;
    pub const BEGIN_SEQ_NO: u32 = 7;
    pub const END_SEQ_NO: u32 = 16;
    pub const TEXT: u32 = 58;
    pub const ENCRYPT_METHOD: u32 = 98;
    pub const HEART_BT_INT: u32 = 108;
    pub const TEST_REQ_ID: u32 = 112;
    pub const NEW_SEQ_NO: u32 = 36;
    pub const GAP_FILL_FLAG: u32 = 123;
    pub const RESET_SEQ_NUM_FLAG: u32 = 141;
    pub const MD_REQ_ID: u32 = 262;
    pub const SUBSCRIPTION_TYPE: u32 = 263;
    pub const MARKET_DEPTH: u32 = 264;
    pub const MD_UPDATE_TYPE: u32 = 265;
    pub const NO_MD_ENTRY_TYPES: u32 = 267;
    pub const MD_ENTRY_TYPE: u32 = 269;
    pub const NO_RELATED_SYM: u32 = 146;
    pub const SYMBOL: u32 = 55;
}

/// `263` MDSubscriptionRequestType.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SubscriptionType {
    /// `0` — one snapshot, then nothing.
    Snapshot,

    /// `1` — a snapshot and every update after it. What a feed handler wants.
    #[default]
    SnapshotPlusUpdates,

    /// `2` — cancel a running subscription.
    Unsubscribe,
}

impl SubscriptionType {
    /// The `263` byte.
    #[inline]
    pub const fn as_byte(self) -> u8 {
        match self {
            Self::Snapshot => b'0',
            Self::SnapshotPlusUpdates => b'1',
            Self::Unsubscribe => b'2',
        }
    }
}

/// What to ask the venue for, in one `35=V`.
///
/// **This is the one message in the crate that is a venue conversation rather
/// than a protocol obligation**, which is why the receive loop never sends it
/// on its own: what depth a venue supports, whether it wants incremental
/// updates, and which entry types it will honour are things only the deployment
/// knows. The binary sends it after logon (`documents/todo.md` §10).
#[derive(Debug, Clone, Copy)]
pub struct SubscriptionRequest<'a> {
    /// `262` MDReqID — echoed back on every message that answers this request.
    pub req_id: &'a [u8],

    /// `263` subscribe, snapshot-once, or cancel.
    pub subscription: SubscriptionType,

    /// `264` MarketDepth. `0` is the full book, `1` is top of book.
    pub depth: u32,

    /// `265` MDUpdateType: `0` full refresh, `1` incremental. `None` leaves it
    /// to the venue's default, which is what a venue that only speaks `35=W`
    /// needs.
    pub update_type: Option<u8>,

    /// `267`/`269` the entry types wanted — usually bid, offer and trade.
    pub entry_types: &'a [MdEntryType],

    /// `146`/`55` the instruments wanted.
    pub symbols: &'a [&'a [u8]],
}

/// A message did not fit the buffer it was asked to write into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmitError {
    /// Size of the buffer that was too small.
    pub capacity: usize,
}

impl fmt::Display for EmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FIX message does not fit in {} bytes", self.capacity)
    }
}

impl std::error::Error for EmitError {}

/// Writes the outbound side of one session.
///
/// Owns the outbound `MsgSeqNum`, which is the counterparty's only way to
/// notice *our* loss, and the time we last spoke, which is what decides when a
/// heartbeat is due.
#[derive(Debug, Clone)]
pub struct Emitter {
    begin_string: [u8; 16],
    begin_len: u8,
    sender: FixText,
    target: FixText,
    next_seq: u64,
    last_sent_ns: UnixNano,
    body: [u8; MAX_EMIT_LEN],
}

impl Emitter {
    /// A session that will introduce itself as `sender` to `target`,
    /// starting at `MsgSeqNum` 1.
    pub fn new(sender: &[u8], target: &[u8]) -> Self {
        let mut begin_string = [0u8; 16];
        let n = FIX_BEGIN_STRING.len().min(16);
        begin_string[..n].copy_from_slice(&FIX_BEGIN_STRING[..n]);
        Self {
            begin_string,
            begin_len: n as u8,
            sender: FixText::new(sender),
            target: FixText::new(target),
            next_seq: 1,
            last_sent_ns: 0,
            body: [0; MAX_EMIT_LEN],
        }
    }

    /// Speaks a different `BeginString` — a venue still on FIX.4.2, say.
    ///
    /// The decoder does not check it either ([`frame`](crate::frame())): framing
    /// has to succeed before a version policy can be applied to what it framed.
    pub fn with_begin_string(mut self, begin_string: &[u8]) -> Self {
        let n = begin_string.len().min(16);
        self.begin_string = [0; 16];
        self.begin_string[..n].copy_from_slice(&begin_string[..n]);
        self.begin_len = n as u8;
        self
    }

    /// `SenderCompID` (49) — us.
    #[inline]
    pub fn sender(&self) -> &[u8] {
        self.sender.as_bytes()
    }

    /// `TargetCompID` (56) — the venue.
    #[inline]
    pub fn target(&self) -> &[u8] {
        self.target.as_bytes()
    }

    /// The number the next outbound message will carry.
    #[inline]
    pub const fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// When we last sent anything. Zero before the first message.
    #[inline]
    pub const fn last_sent_ns(&self) -> UnixNano {
        self.last_sent_ns
    }

    /// `true` when we have said nothing for `interval_ns` and owe the venue a
    /// heartbeat.
    ///
    /// `saturating_sub` because [`UnixNano`] is unsigned and the system clock
    /// is not monotonic (`CLAUDE.md`).
    #[inline]
    pub const fn owes_heartbeat(&self, now: UnixNano, interval_ns: u64) -> bool {
        if interval_ns == 0 || self.last_sent_ns == 0 {
            return false;
        }
        now.saturating_sub(self.last_sent_ns) >= interval_ns
    }

    /// Restarts the outbound sequence at 1, for a new connection.
    #[inline]
    pub fn reset(&mut self) {
        self.next_seq = 1;
        self.last_sent_ns = 0;
    }

    /// Forces the next outbound number, for a session resumed out of band.
    #[inline]
    pub fn set_next_seq(&mut self, seq: u64) {
        self.next_seq = seq;
    }

    /// `35=A` Logon.
    ///
    /// `reset_seq` sends `141=Y`, which tells the venue to start both
    /// directions at 1. A market-data session wants that on every connect: we
    /// keep no book across a disconnect, so resuming a sequence would only
    /// invite a resend of messages describing a market that has since moved.
    pub fn logon(
        &mut self,
        heartbeat_secs: u32,
        reset_seq: bool,
        now: UnixNano,
        out: &mut [u8],
    ) -> Result<usize, EmitError> {
        self.build(b"A", now, out, |w| {
            w.tag_u64(tags::ENCRYPT_METHOD, 0);
            w.tag_u64(tags::HEART_BT_INT, u64::from(heartbeat_secs));
            if reset_seq {
                w.tag(tags::RESET_SEQ_NUM_FLAG, b"Y");
            }
        })
    }

    /// `35=0` Heartbeat. With `test_req_id`, it is the answer to a
    /// `TestRequest` and must echo the id it carried.
    pub fn heartbeat(
        &mut self,
        test_req_id: Option<&[u8]>,
        now: UnixNano,
        out: &mut [u8],
    ) -> Result<usize, EmitError> {
        self.build(b"0", now, out, |w| {
            if let Some(id) = test_req_id {
                w.tag(tags::TEST_REQ_ID, id);
            }
        })
    }

    /// `35=1` TestRequest — "are you there", after one interval of silence.
    pub fn test_request(
        &mut self,
        id: &[u8],
        now: UnixNano,
        out: &mut [u8],
    ) -> Result<usize, EmitError> {
        self.build(b"1", now, out, |w| w.tag(tags::TEST_REQ_ID, id))
    }

    /// `35=2` ResendRequest for `begin..=end`. `end` of `0` means "everything
    /// from `begin` onward", as FIX encodes it.
    pub fn resend_request(
        &mut self,
        begin: u64,
        end: u64,
        now: UnixNano,
        out: &mut [u8],
    ) -> Result<usize, EmitError> {
        self.build(b"2", now, out, |w| {
            w.tag_u64(tags::BEGIN_SEQ_NO, begin);
            w.tag_u64(tags::END_SEQ_NO, end);
        })
    }

    /// `35=4` SequenceReset, normally with `123=Y` (GapFill).
    ///
    /// This is how a market-data consumer answers a `ResendRequest`: everything
    /// we send is session administration, and replaying a Heartbeat from ten
    /// minutes ago would be worse than useless. GapFill says "those numbers
    /// existed and carried nothing you want" — which is the truth.
    pub fn sequence_reset(
        &mut self,
        new_seq: u64,
        gap_fill: bool,
        now: UnixNano,
        out: &mut [u8],
    ) -> Result<usize, EmitError> {
        self.build(b"4", now, out, |w| {
            w.tag_u64(tags::NEW_SEQ_NO, new_seq);
            if gap_fill {
                w.tag(tags::GAP_FILL_FLAG, b"Y");
            }
        })
    }

    /// `35=5` Logout, with an optional `58` saying why.
    ///
    /// The reason is worth sending: on the venue's side a logout with no text
    /// is indistinguishable from a handler that crashed, and the difference is
    /// the first thing anyone asks about afterwards.
    pub fn logout(
        &mut self,
        text: Option<&[u8]>,
        now: UnixNano,
        out: &mut [u8],
    ) -> Result<usize, EmitError> {
        self.build(b"5", now, out, |w| {
            if let Some(text) = text {
                w.tag(tags::TEXT, text);
            }
        })
    }

    /// `35=V` MarketDataRequest.
    pub fn market_data_request(
        &mut self,
        req: &SubscriptionRequest<'_>,
        now: UnixNano,
        out: &mut [u8],
    ) -> Result<usize, EmitError> {
        self.build(b"V", now, out, |w| {
            w.tag(tags::MD_REQ_ID, req.req_id);
            w.tag(tags::SUBSCRIPTION_TYPE, &[req.subscription.as_byte()]);
            w.tag_u64(tags::MARKET_DEPTH, u64::from(req.depth));
            if let Some(update) = req.update_type {
                w.tag(tags::MD_UPDATE_TYPE, &[update]);
            }
            w.tag_u64(tags::NO_MD_ENTRY_TYPES, req.entry_types.len() as u64);
            for entry in req.entry_types {
                w.tag(tags::MD_ENTRY_TYPE, &[entry.as_byte()]);
            }
            w.tag_u64(tags::NO_RELATED_SYM, req.symbols.len() as u64);
            for symbol in req.symbols {
                w.tag(tags::SYMBOL, symbol);
            }
        })
    }

    /// Writes `35=<msg_type>` with the standard header, `fields`, and the
    /// trailer, consuming one outbound sequence number.
    ///
    /// `BodyLength` and `CheckSum` are computed from the bytes actually
    /// written, never from an expected length — the same reason
    /// [`frame`](crate::frame::frame) checks them before reading a field.
    fn build(
        &mut self,
        msg_type: &[u8],
        now: UnixNano,
        out: &mut [u8],
        fields: impl FnOnce(&mut Writer<'_>),
    ) -> Result<usize, EmitError> {
        let seq = self.next_seq;
        let sender = self.sender;
        let target = self.target;

        let body_len = {
            let mut w = Writer::new(&mut self.body);
            w.tag(tags::MSG_TYPE, msg_type);
            w.tag(tags::SENDER_COMP_ID, sender.as_bytes());
            w.tag(tags::TARGET_COMP_ID, target.as_bytes());
            w.tag_u64(tags::MSG_SEQ_NUM, seq);
            let mut stamp = [0u8; 21];
            let n = write_sending_time(now, &mut stamp);
            w.tag(tags::SENDING_TIME, &stamp[..n]);
            fields(&mut w);
            w.finish().ok_or(EmitError { capacity: MAX_EMIT_LEN })?
        };

        let mut w = Writer::new(out);
        w.bytes(b"8=");
        w.bytes(&self.begin_string[..self.begin_len as usize]);
        w.byte(SOH);
        w.tag_u64(9, body_len as u64);
        w.bytes(&self.body[..body_len]);
        let head = w.finish().ok_or(EmitError { capacity: out.len() })?;

        // The checksum covers everything before `10=`, which is exactly what
        // has been written so far.
        let sum = checksum(&out[..head]);
        let mut w = Writer::at(out, head);
        w.bytes(b"10=");
        w.byte(b'0' + (sum / 100) % 10);
        w.byte(b'0' + (sum / 10) % 10);
        w.byte(b'0' + sum % 10);
        w.byte(SOH);
        let total = w.finish().ok_or(EmitError { capacity: out.len() })?;

        self.next_seq += 1;
        self.last_sent_ns = now;
        Ok(total)
    }
}

/// Writes `YYYYMMDD-HH:MM:SS.mmm` into `out`, returning its length.
///
/// Milliseconds, not nanoseconds: FIX 4.4 allows more precision, but this field
/// is `SendingTime` and it is *our* clock — the venue's clock arrives in
/// `272`/`273` and is the only one anything downstream reads
/// (`documents/feed_handler.md` §13). Three digits is what a counterparty log
/// can be matched on.
fn write_sending_time(now: UnixNano, out: &mut [u8; 21]) -> usize {
    let secs = now / NS_PER_SEC;
    let millis = (now % NS_PER_SEC) / 1_000_000;
    let days = (secs / 86_400) as i64;
    let sod = secs % 86_400;
    let (y, m, d) = civil_from_days(days);

    let mut w = Writer::new(out);
    w.pad(y.rem_euclid(10_000) as u64, 4);
    w.pad(u64::from(m), 2);
    w.pad(u64::from(d), 2);
    w.byte(b'-');
    w.pad(sod / 3_600, 2);
    w.byte(b':');
    w.pad((sod / 60) % 60, 2);
    w.byte(b':');
    w.pad(sod % 60, 2);
    w.byte(b'.');
    w.pad(millis, 3);
    w.finish().unwrap_or(0)
}

/// `(year, month, day)` from days since 1970-01-01 — Howard Hinnant's
/// `civil_from_days`, the inverse of
/// [`days_from_civil`](crate::tagvalue::days_from_civil).
///
/// The pair is kept as two independent implementations of the same table on
/// purpose: a round trip through both is a real check, and the decoder's tests
/// already pin the forward direction against the capture.
#[inline]
pub const fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Appends bytes to a fixed buffer, remembering whether it ran out.
///
/// Overflow is sticky rather than checked at every call: a message that did not
/// fit is one error at the end, and the alternative is a `?` on every field of
/// every builder.
struct Writer<'a> {
    buf: &'a mut [u8],
    len: usize,
    full: bool,
}

impl<'a> Writer<'a> {
    #[inline]
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, len: 0, full: false }
    }

    /// Continues writing at `at`, leaving what is already there.
    #[inline]
    fn at(buf: &'a mut [u8], at: usize) -> Self {
        let full = at > buf.len();
        let len = at.min(buf.len());
        Self { buf, len, full }
    }

    #[inline]
    fn byte(&mut self, b: u8) {
        if self.len < self.buf.len() {
            self.buf[self.len] = b;
            self.len += 1;
        } else {
            self.full = true;
        }
    }

    #[inline]
    fn bytes(&mut self, bytes: &[u8]) {
        if self.len + bytes.len() > self.buf.len() {
            self.full = true;
            return;
        }
        self.buf[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
    }

    /// `value` in decimal, no padding.
    #[inline]
    fn u64(&mut self, value: u64) {
        let mut digits = [0u8; 20];
        let mut n = 0;
        let mut v = value;
        loop {
            digits[n] = b'0' + (v % 10) as u8;
            n += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        for i in (0..n).rev() {
            self.byte(digits[i]);
        }
    }

    /// `value` in decimal, zero-padded to `width`.
    #[inline]
    fn pad(&mut self, value: u64, width: usize) {
        let mut digits = [0u8; 20];
        let mut n = 0;
        let mut v = value;
        loop {
            digits[n] = b'0' + (v % 10) as u8;
            n += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        for _ in n..width {
            self.byte(b'0');
        }
        for i in (0..n).rev() {
            self.byte(digits[i]);
        }
    }

    /// `tag=value<SOH>`.
    #[inline]
    fn tag(&mut self, tag: u32, value: &[u8]) {
        self.u64(u64::from(tag));
        self.byte(b'=');
        self.bytes(value);
        self.byte(SOH);
    }

    /// `tag=<decimal><SOH>`.
    #[inline]
    fn tag_u64(&mut self, tag: u32, value: u64) {
        self.u64(u64::from(tag));
        self.byte(b'=');
        self.u64(value);
        self.byte(SOH);
    }

    /// Bytes written, or `None` if anything was dropped.
    #[inline]
    fn finish(self) -> Option<usize> {
        if self.full { None } else { Some(self.len) }
    }
}
