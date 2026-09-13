//! Sequence and liveness bookkeeping for one FIX session.
//!
//! FIX is the mirror image of KRX multicast (`documents/feed_handler.md`
//! §10): TCP removes loss and reordering, so a sequence gap is not a normal
//! condition to be healed by the next snapshot — it is a session incident
//! that needs a `ResendRequest`. That is why this logic lives apart from the
//! KRX path and why nothing here is shared with it.
//!
//! This module is pure bookkeeping over already-framed messages. It opens no
//! socket, sends nothing, and spawns no thread: it answers "what happened to
//! this session" and leaves the reaction to the handler that owns the
//! connection.

use crate::error::FixError;
use crate::frame::Frame;
use crate::market_data::{tags, FixText};
use crate::tagvalue::MsgType;
use jeed_wire::UnixNano;

/// Session-administration tags this module reads.
pub mod admin_tags {
    /// BeginSeqNo (ResendRequest).
    pub const BEGIN_SEQ_NO: u32 = 7;
    /// EndSeqNo (ResendRequest).
    pub const END_SEQ_NO: u32 = 16;
    /// NewSeqNo (SequenceReset).
    pub const NEW_SEQ_NO: u32 = 36;
    /// HeartBtInt in seconds (Logon).
    pub const HEART_BT_INT: u32 = 108;
    /// TestReqID (TestRequest / Heartbeat).
    pub const TEST_REQ_ID: u32 = 112;
    /// GapFillFlag (SequenceReset).
    pub const GAP_FILL_FLAG: u32 = 123;
}

/// Reads `35=` without decoding the rest of the message.
///
/// Used to route a frame to the market-data decoder or the session decoder
/// before paying for either.
pub fn msg_type(frame: &Frame<'_>) -> Result<MsgType, FixError> {
    for field in frame.fields() {
        let field = field?;
        if field.tag == tags::MSG_TYPE {
            return Ok(MsgType::from_value(field.value));
        }
    }
    Err(FixError::MissingTag { tag: tags::MSG_TYPE })
}

/// A decoded session-administration message (`35` = `0 1 2 3 4 5 A`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdminMessage {
    /// `35` MsgType.
    pub msg_type: MsgType,

    /// `34` MsgSeqNum.
    pub msg_seq_num: u64,

    /// `52` SendingTime.
    pub sending_time: UnixNano,

    /// `43` PossDupFlag.
    pub poss_dup: bool,

    /// `123` GapFillFlag on a SequenceReset.
    pub gap_fill: bool,

    /// `108` HeartBtInt in seconds (Logon), `None` when absent.
    pub heartbeat_secs: Option<u32>,

    /// `36` NewSeqNo (SequenceReset), `None` when absent.
    pub new_seq_no: Option<u64>,

    /// `7` BeginSeqNo (ResendRequest), `None` when absent.
    pub begin_seq_no: Option<u64>,

    /// `16` EndSeqNo (ResendRequest; `0` means "infinity"), `None` when absent.
    pub end_seq_no: Option<u64>,

    /// `112` TestReqID.
    pub test_req_id: FixText,
}

impl Default for AdminMessage {
    #[inline]
    fn default() -> Self {
        Self::EMPTY
    }
}

impl AdminMessage {
    /// Message with nothing decoded.
    pub const EMPTY: Self = Self {
        msg_type: MsgType::Other { first: 0 },
        msg_seq_num: 0,
        sending_time: 0,
        poss_dup: false,
        gap_fill: false,
        heartbeat_secs: None,
        new_seq_no: None,
        begin_seq_no: None,
        end_seq_no: None,
        test_req_id: FixText::EMPTY,
    };
}

/// Decodes a session-administration message.
pub fn parse_admin_message(frame: &Frame<'_>) -> Result<AdminMessage, FixError> {
    let mut msg = AdminMessage::EMPTY;
    let mut have_msg_type = false;
    let mut have_seq = false;
    for field in frame.fields() {
        let field = field?;
        match field.tag {
            tags::MSG_TYPE => {
                msg.msg_type = MsgType::from_value(field.value);
                if !msg.msg_type.is_admin() {
                    return Err(FixError::UnexpectedMsgType {
                        found: field.value.first().copied().unwrap_or(0),
                    });
                }
                have_msg_type = true;
            }
            tags::MSG_SEQ_NUM => {
                msg.msg_seq_num = field.as_u64()?;
                have_seq = true;
            }
            tags::SENDING_TIME => msg.sending_time = field.as_utc_timestamp()?,
            tags::POSS_DUP_FLAG => msg.poss_dup = field.is_yes(),
            admin_tags::GAP_FILL_FLAG => msg.gap_fill = field.is_yes(),
            admin_tags::HEART_BT_INT => msg.heartbeat_secs = Some(field.as_u32()?),
            admin_tags::NEW_SEQ_NO => msg.new_seq_no = Some(field.as_u64()?),
            admin_tags::BEGIN_SEQ_NO => msg.begin_seq_no = Some(field.as_u64()?),
            admin_tags::END_SEQ_NO => msg.end_seq_no = Some(field.as_u64()?),
            admin_tags::TEST_REQ_ID => msg.test_req_id = FixText::new(field.value),
            _ => {}
        }
    }
    if !have_msg_type {
        return Err(FixError::MissingTag { tag: tags::MSG_TYPE });
    }
    if !have_seq {
        return Err(FixError::MissingTag { tag: tags::MSG_SEQ_NUM });
    }
    Ok(msg)
}

/// What the sequence number of an incoming message means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeqVerdict {
    /// The expected number. Process the message.
    InOrder,

    /// Numbers are missing. The handler owes the counterparty a
    /// `ResendRequest` for [`Self::Gap::missing`] starting at
    /// [`Self::Gap::expected`].
    Gap {
        /// Number that was expected.
        expected: u64,

        /// Number that arrived.
        received: u64,

        /// How many messages were skipped.
        missing: u64,
    },

    /// A number already seen, with `PossDupFlag=Y` — a legitimate resend.
    /// Drop the message; it changes no state.
    Duplicate {
        /// Number that was expected.
        expected: u64,

        /// Number that arrived.
        received: u64,
    },

    /// A number already seen *without* `PossDupFlag` — a protocol error the
    /// spec answers with a logout, not a skip.
    Regression {
        /// Number that was expected.
        expected: u64,

        /// Number that arrived.
        received: u64,
    },
}

impl SeqVerdict {
    /// `true` only for [`Self::InOrder`].
    #[inline]
    pub const fn is_in_order(self) -> bool {
        matches!(self, Self::InOrder)
    }

    /// The inclusive `(BeginSeqNo, EndSeqNo)` to ask back, where `0` as the
    /// end means "everything from here" as FIX encodes it.
    #[inline]
    pub const fn resend_range(self) -> Option<(u64, u64)> {
        match self {
            Self::Gap { expected, received, .. } => Some((expected, received - 1)),
            _ => None,
        }
    }
}

/// Sequence and liveness state of one inbound FIX session.
///
/// A gap **advances** the expected number past the hole: our market-data
/// path is snapshot-driven, so refusing to move on would stall the book
/// behind a resend we may never get. The hole is counted
/// ([`Self::lost`]) and reported once as [`SeqVerdict::Gap`]; requesting
/// the resend is the caller's decision, not a side effect of observing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionState {
    /// Sequence number the next message should carry.
    pub next_expected: u64,

    /// Reception time of the last framed message — the handler's own
    /// `recv_ns`, never a clock the counterparty sent.
    pub last_recv_ns: UnixNano,

    /// Negotiated heartbeat interval in ns (`108` × 1e9); `0` until Logon.
    pub heartbeat_interval_ns: u64,

    /// Messages observed, including duplicates.
    pub observed: u64,

    /// Gap events seen.
    pub gaps: u64,

    /// Messages skipped in total.
    pub lost: u64,

    /// `PossDupFlag` resends dropped.
    pub duplicates: u64,

    /// Backwards jumps without `PossDupFlag`.
    pub regressions: u64,

    /// A Logon has been observed.
    pub logged_on: bool,
}

impl Default for SessionState {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl SessionState {
    /// Fresh session expecting sequence number 1.
    pub const fn new() -> Self {
        Self {
            next_expected: 1,
            last_recv_ns: 0,
            heartbeat_interval_ns: 0,
            observed: 0,
            gaps: 0,
            lost: 0,
            duplicates: 0,
            regressions: 0,
            logged_on: false,
        }
    }

    /// Judges one message's sequence number and folds it into the state.
    ///
    /// `poss_dup` is `43=Y`; `recv_ns` is the handler's reception time.
    pub fn observe(&mut self, seq: u64, poss_dup: bool, recv_ns: UnixNano) -> SeqVerdict {
        self.observed += 1;
        self.last_recv_ns = recv_ns;
        let expected = self.next_expected;
        if seq == expected {
            self.next_expected = seq + 1;
            return SeqVerdict::InOrder;
        }
        if seq > expected {
            let missing = seq - expected;
            self.gaps += 1;
            self.lost += missing;
            self.next_expected = seq + 1;
            return SeqVerdict::Gap { expected, received: seq, missing };
        }
        if poss_dup {
            self.duplicates += 1;
            SeqVerdict::Duplicate { expected, received: seq }
        } else {
            self.regressions += 1;
            SeqVerdict::Regression { expected, received: seq }
        }
    }

    /// Applies an administration message's session effects: Logon fixes the
    /// heartbeat interval, `SequenceReset` moves the expected number.
    ///
    /// The message's own sequence number is **not** consumed here — call
    /// [`observe`](Self::observe) for that, as with any other message
    /// (a `SequenceReset` with `GapFillFlag=Y` is a real message and
    /// occupies its number; a reset without it is administrative).
    pub fn apply_admin(&mut self, msg: &AdminMessage) {
        if let MsgType::Logon = msg.msg_type {
            self.logged_on = true;
            if let Some(secs) = msg.heartbeat_secs {
                self.heartbeat_interval_ns = u64::from(secs) * 1_000_000_000;
            }
        }
        if let MsgType::Logout = msg.msg_type {
            self.logged_on = false;
        }
        if let (MsgType::SequenceReset, Some(new_seq)) = (msg.msg_type, msg.new_seq_no) {
            self.next_expected = new_seq;
        }
    }

    /// Forces the expected number, for a session restarted out of band.
    #[inline]
    pub fn reset_to(&mut self, seq: u64) {
        self.next_expected = seq;
    }

    /// Nanoseconds since the last framed message. Saturating: neither clock
    /// is monotonic.
    #[inline]
    pub fn silence_ns(&self, now: UnixNano) -> UnixNano {
        now.saturating_sub(self.last_recv_ns)
    }

    /// `true` when nothing has arrived for `factor` heartbeat intervals.
    ///
    /// FIX 4.4 says to send a `TestRequest` after one interval and to
    /// consider the session dead after a second one, so `factor` is the
    /// caller's escalation step (1 → test, 2 → tear down). Always `false`
    /// before a heartbeat interval is negotiated or a first message seen.
    #[inline]
    pub fn is_silent(&self, now: UnixNano, factor: u32) -> bool {
        if self.heartbeat_interval_ns == 0 || self.last_recv_ns == 0 {
            return false;
        }
        self.silence_ns(now) > self.heartbeat_interval_ns * u64::from(factor.max(1))
    }
}
