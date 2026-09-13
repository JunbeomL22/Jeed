//! Replaying a packet capture through the KRX pipeline.
//!
//! ```text
//! capture.pcap ──→ Reader ──→ datagram() ──→ Pipeline::ingest ──→ Replay
//!                  libpcap     eth·ip·udp     the receive loop's     │
//!                                                                    ├──→ report
//!                                                                    └──→ records.tsv
//! ```
//!
//! The decoder tests are built from the standard, and a standard misread is a
//! test that passes — `B604F` sat on the five-deep side for a reading of the
//! channel standard while the circuit sent 554 bytes, and every one of the
//! day's 31 million was refused as wrong-length with 141 decoder tests green
//! (`documents/todo.md` §16). A layout read one column off still parses, and
//! parses into plausible numbers. The only ground truth is a day of
//! the circuit itself, and `jeed-pcap` is how a capture of one is put through
//! the handler: every datagram takes the **same path it would take live** —
//! [`Pipeline::ingest`] with the same filters — and what comes out is tallied
//! by trcode, with the first refused messages kept with their cause and head.
//! The records themselves can be written out one line each, which is what
//! `documents/todo.md` §4 compares against the reference decoder's output.
//!
//! No `pcap` crate. The classic file format is a 24-byte header and a 16-byte
//! record header per packet; the frames on a market-data tap are plain
//! Ethernet, IPv4, UDP, occasionally behind a VLAN tag. That is [`Reader`] and
//! [`datagram`], a hundred lines on the cold path, and the workspace keeps its
//! one external dependency.

use crate::log::Timestamp;
use core::fmt;
use jeed_krx::recv::{IsinFilter, Outcome, Pipeline, Stats, TrCodeFilter};
use jeed_krx::{END_KEYWORD, KrxError, TRCODE_LEN, TrCode};
use jeed_wire::{QuotePayload, RecordSink, TradePayload, UnixNano, WireKind, WireRecord};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::path::Path;

// ===========================================================================
// The file
// ===========================================================================

/// Magic of a capture with microsecond timestamps, as written by `tcpdump`.
pub const MAGIC_MICROS: u32 = 0xa1b2_c3d4;

/// Magic of a capture with nanosecond timestamps.
pub const MAGIC_NANOS: u32 = 0xa1b2_3c4d;

/// The only link type read: Ethernet.
pub const LINK_ETHERNET: u32 = 1;

/// Largest captured length accepted for one packet.
///
/// `tcpdump` writes a snapshot length of 262,144; a record header claiming
/// more than a few times that is a corrupt header, not a jumbo frame, and
/// reading it would mean allocating whatever the four bytes happened to say.
pub const MAX_CAPTURED: usize = 1 << 20;

/// A libpcap file, one packet at a time.
///
/// The buffer is reused between packets: [`next_packet`](Self::next_packet) hands out a
/// borrow of it, so a packet is looked at and dropped before the next one is
/// read. That is all a replay needs, and it makes ninety gigabytes cost one
/// megabyte of heap.
#[derive(Debug)]
pub struct Reader<R> {
    src: R,
    swap: bool,
    nanos: bool,
    buf: Vec<u8>,
    packets: u64,
}

/// One captured packet.
#[derive(Debug, Clone, Copy)]
pub struct Packet<'a> {
    /// Capture time as Unix nanoseconds — what the replay stamps as `recv_ns`.
    pub ts_ns: UnixNano,

    /// Length on the wire. Larger than `bytes` when the snapshot cut it.
    pub original: usize,

    /// The captured bytes, starting at the Ethernet header.
    pub bytes: &'a [u8],
}

impl Packet<'_> {
    /// `true` when the capture kept fewer bytes than were on the wire.
    #[inline]
    pub const fn truncated(&self) -> bool {
        self.bytes.len() < self.original
    }
}

impl Reader<BufReader<File>> {
    /// Opens a capture file.
    pub fn open(path: &Path) -> Result<Self, PcapError> {
        let file = File::open(path).map_err(PcapError::Io)?;
        Self::new(BufReader::with_capacity(1 << 20, file))
    }
}

impl<R: Read> Reader<R> {
    /// Reads the file header and refuses anything that is not Ethernet frames
    /// in the classic format.
    pub fn new(mut src: R) -> Result<Self, PcapError> {
        let mut head = [0u8; 24];
        match read_full(&mut src, &mut head) {
            Ok(24) => {}
            Ok(_) => return Err(PcapError::Truncated { packet: 0 }),
            Err(e) => return Err(PcapError::Io(e)),
        }
        let magic = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let (swap, nanos) = match magic {
            MAGIC_MICROS => (false, false),
            MAGIC_NANOS => (false, true),
            m if m.swap_bytes() == MAGIC_MICROS => (true, false),
            m if m.swap_bytes() == MAGIC_NANOS => (true, true),
            other => return Err(PcapError::Magic(other)),
        };
        let link = word(&head[20..24], swap);
        if link != LINK_ETHERNET {
            return Err(PcapError::LinkType(link));
        }
        Ok(Self { src, swap, nanos, buf: Vec::new(), packets: 0 })
    }

    /// The next packet, or `None` at a clean end of file.
    ///
    /// A file that ends inside a packet — a capture that was killed rather
    /// than stopped, which is the normal end of a day's tap — is
    /// [`PcapError::Truncated`], after every whole packet before it has been
    /// returned.
    pub fn next_packet(&mut self) -> Result<Option<Packet<'_>>, PcapError> {
        let mut head = [0u8; 16];
        match read_full(&mut self.src, &mut head) {
            Ok(0) => return Ok(None),
            Ok(16) => {}
            Ok(_) => return Err(PcapError::Truncated { packet: self.packets }),
            Err(e) => return Err(PcapError::Io(e)),
        }
        let secs = word(&head[0..4], self.swap);
        let frac = word(&head[4..8], self.swap);
        let captured = word(&head[8..12], self.swap) as usize;
        let original = word(&head[12..16], self.swap) as usize;
        if captured > MAX_CAPTURED {
            return Err(PcapError::Length { packet: self.packets, captured });
        }

        self.buf.resize(captured, 0);
        match read_full(&mut self.src, &mut self.buf) {
            Ok(n) if n == captured => {}
            Ok(_) => return Err(PcapError::Truncated { packet: self.packets }),
            Err(e) => return Err(PcapError::Io(e)),
        }

        let unit = if self.nanos { 1 } else { 1_000 };
        let ts_ns = u64::from(secs) * 1_000_000_000 + u64::from(frac) * unit;
        self.packets += 1;
        Ok(Some(Packet { ts_ns, original, bytes: &self.buf }))
    }

    /// Packets returned so far.
    #[inline]
    pub const fn packets(&self) -> u64 {
        self.packets
    }
}

/// A four-byte field in the file's byte order.
#[inline]
fn word(bytes: &[u8], swap: bool) -> u32 {
    let v = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    if swap { v.swap_bytes() } else { v }
}

/// Fills `buf` as far as the source allows; the count says how far.
fn read_full(src: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match src.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// Why a capture could not be read.
#[derive(Debug)]
pub enum PcapError {
    /// The file could not be read.
    Io(io::Error),

    /// Not a libpcap file. The value is the first four bytes.
    Magic(u32),

    /// A libpcap file of something other than Ethernet frames.
    LinkType(u32),

    /// The file ends inside packet `packet` (zero-based; `0` with no packet
    /// read means the file header itself is short).
    Truncated {
        /// Index of the packet the file ends inside.
        packet: u64,
    },

    /// A packet header claims more than [`MAX_CAPTURED`] bytes.
    Length {
        /// Index of the packet.
        packet: u64,

        /// The claimed captured length.
        captured: usize,
    },
}

impl fmt::Display for PcapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "read failed: {e}"),
            Self::Magic(m) => write!(f, "not a libpcap file (magic {m:#010x})"),
            Self::LinkType(l) => write!(f, "link type {l} is not Ethernet ({LINK_ETHERNET})"),
            Self::Truncated { packet } => write!(f, "file ends inside packet {packet}"),
            Self::Length { packet, captured } => {
                write!(f, "packet {packet} claims {captured} bytes, more than {MAX_CAPTURED}")
            }
        }
    }
}

impl std::error::Error for PcapError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

// ===========================================================================
// The frame
// ===========================================================================

/// One UDP datagram found inside an Ethernet frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Datagram<'a> {
    /// IPv4 destination — the multicast group.
    pub dst: [u8; 4],

    /// UDP destination port.
    pub port: u16,

    /// The datagram's payload: what the socket would have handed the loop.
    pub payload: &'a [u8],
}

/// Why a frame carried no datagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// Not IPv4 — ARP, IPv6, spanning tree, whatever else shares the tap.
    NotIpv4,

    /// IPv4 but not UDP.
    NotUdp,

    /// A fragment. Market data is never fragmented; a fragment on the tap is
    /// something else's traffic, and reassembling it is not this tool's job.
    Fragment,

    /// Shorter than its own headers say, usually a truncated capture.
    Short,
}

impl fmt::Display for Skip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotIpv4 => "not IPv4",
            Self::NotUdp => "not UDP",
            Self::Fragment => "IP fragment",
            Self::Short => "shorter than its headers",
        })
    }
}

/// Unwraps Ethernet (with any VLAN tags), IPv4 and UDP.
///
/// The payload length comes from the UDP header, not the frame: a short
/// datagram is padded to the Ethernet minimum, and the padding is not KRX's.
pub fn datagram(frame: &[u8]) -> Result<Datagram<'_>, Skip> {
    if frame.len() < 14 {
        return Err(Skip::Short);
    }
    let mut at = 12;
    let mut ethertype = be16(frame, at);
    while ethertype == 0x8100 || ethertype == 0x88a8 {
        at += 4;
        if frame.len() < at + 2 {
            return Err(Skip::Short);
        }
        ethertype = be16(frame, at);
    }
    at += 2;
    if ethertype != 0x0800 {
        return Err(Skip::NotIpv4);
    }

    let ip = &frame[at..];
    if ip.len() < 20 {
        return Err(Skip::Short);
    }
    if ip[0] >> 4 != 4 {
        return Err(Skip::NotIpv4);
    }
    let ihl = usize::from(ip[0] & 0x0f) * 4;
    if ihl < 20 || ip.len() < ihl {
        return Err(Skip::Short);
    }
    if ip[9] != 17 {
        return Err(Skip::NotUdp);
    }
    // More-fragments bit or a non-zero offset: either way not a whole datagram.
    if be16(ip, 6) & 0x3fff != 0 {
        return Err(Skip::Fragment);
    }
    let dst = [ip[16], ip[17], ip[18], ip[19]];

    let udp = &ip[ihl..];
    if udp.len() < 8 {
        return Err(Skip::Short);
    }
    let port = be16(udp, 2);
    let len = usize::from(be16(udp, 4));
    if len < 8 || udp.len() < len {
        return Err(Skip::Short);
    }
    Ok(Datagram { dst, port, payload: &udp[8..len] })
}

#[inline]
fn be16(bytes: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([bytes[at], bytes[at + 1]])
}

// ===========================================================================
// The replay
// ===========================================================================

/// Bytes of a refused message kept for the report.
pub const FAILURE_HEAD: usize = 64;

/// Distinct failures kept per trcode.
pub const FAILURES_KEPT: usize = 8;

/// What the datagrams of one trcode turned into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeTally {
    /// The code.
    pub code: TrCode,

    /// Datagrams carrying it.
    pub received: u64,

    /// Datagram lengths seen, with counts. A code has one interface and one
    /// length; a second entry here is the finding.
    pub lengths: Vec<(usize, u64)>,

    /// Datagrams whose last byte was not the end keyword.
    pub bad_end: u64,

    /// Published to the sink.
    pub published: u64,

    /// Not in the allow-set. Counted, not decoded.
    pub filtered: u64,

    /// In the allow-set, but the instrument was not.
    pub filtered_isin: u64,

    /// In the allow-set, and this build has no decoder for it.
    pub unknown: u64,

    /// In the allow-set, and not the length its interface defines.
    pub wrong_length: u64,

    /// Decoded and refused.
    pub failed: u64,

    /// The first [`FAILURES_KEPT`] distinct refusals, with their heads.
    pub failures: Vec<Failure>,
}

impl CodeTally {
    fn new(code: TrCode) -> Self {
        Self {
            code,
            received: 0,
            lengths: Vec::new(),
            bad_end: 0,
            published: 0,
            filtered: 0,
            filtered_isin: 0,
            unknown: 0,
            wrong_length: 0,
            failed: 0,
            failures: Vec::new(),
        }
    }

    fn saw_length(&mut self, len: usize) {
        match self.lengths.iter_mut().find(|(l, _)| *l == len) {
            Some((_, n)) => *n += 1,
            None => self.lengths.push((len, 1)),
        }
    }

    fn refused(&mut self, error: KrxError, payload: &[u8]) {
        self.failed += 1;
        if let Some(f) = self.failures.iter_mut().find(|f| f.error == error) {
            f.count += 1;
        } else if self.failures.len() < FAILURES_KEPT {
            let mut head = [0u8; FAILURE_HEAD];
            let len = payload.len().min(FAILURE_HEAD);
            head[..len].copy_from_slice(&payload[..len]);
            self.failures.push(Failure { error, count: 1, head, len });
        }
    }
}

/// One kind of refusal and the first message that earned it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The decoder's objection.
    pub error: KrxError,

    /// Messages refused with exactly this error.
    pub count: u64,

    /// The first such message's leading bytes.
    pub head: [u8; FAILURE_HEAD],

    /// How many of `head` are real.
    pub len: usize,
}

impl Failure {
    /// The kept bytes.
    #[inline]
    pub fn head(&self) -> &[u8] {
        &self.head[..self.len]
    }
}

/// What arrived on one multicast group and port.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortTally {
    /// Datagrams.
    pub received: u64,

    /// Datagrams per trcode, by the code's `u64` packing.
    pub codes: BTreeMap<u64, u64>,
}

/// Counts over the whole capture.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PacketTally {
    /// Packets read.
    pub packets: u64,

    /// Packets that were a whole UDP datagram.
    pub datagrams: u64,

    /// Packets the snapshot length cut short.
    pub truncated: u64,

    /// Skipped: not IPv4.
    pub not_ipv4: u64,

    /// Skipped: not UDP.
    pub not_udp: u64,

    /// Skipped: an IP fragment.
    pub fragments: u64,

    /// Skipped: shorter than its headers.
    pub short: u64,

    /// Payload bytes handed to the pipeline.
    pub bytes: u64,

    /// Capture time of the first packet.
    pub first_ns: UnixNano,

    /// Capture time of the last packet.
    pub last_ns: UnixNano,
}

/// The pipeline's sink: keeps the record it was last given, so the replay can
/// write it out after `ingest` says it was published.
#[derive(Debug)]
struct Last {
    record: WireRecord,
    drops: u64,
}

impl RecordSink for Last {
    #[inline]
    fn publish<E>(&mut self, fill: impl FnOnce(&mut WireRecord) -> Result<(), E>) -> Result<(), E> {
        fill(&mut self.record)
    }

    #[inline]
    fn note_drops(&mut self, n: u64) {
        self.drops += n;
    }
}

/// A capture going through the pipeline, and what came of it.
#[derive(Debug)]
pub struct Replay<W> {
    pipeline: Pipeline<Last>,
    codes: BTreeMap<u64, CodeTally>,
    ports: BTreeMap<([u8; 4], u16), PortTally>,
    packets: PacketTally,
    dump: Option<W>,
}

impl<W: Write> Replay<W> {
    /// Builds a replay with the receive loop's filters.
    ///
    /// `dump`, when given, receives one TSV line per published record
    /// ([`write_tsv`]); its header line is written now.
    pub fn new(trcodes: TrCodeFilter, isins: IsinFilter, mut dump: Option<W>) -> io::Result<Self> {
        if let Some(w) = dump.as_mut() {
            write_tsv_header(w)?;
        }
        let sink = Last { record: WireRecord::zeroed(), drops: 0 };
        Ok(Self {
            // Stale is a live policy: against a capture, `recv_ns` is the tap's
            // clock and the age it would compute is the tap's latency, which
            // is a fact about the capture and not something to flag on.
            pipeline: Pipeline::new(trcodes, isins, 0, sink),
            codes: BTreeMap::new(),
            ports: BTreeMap::new(),
            packets: PacketTally::default(),
            dump,
        })
    }

    /// Unwraps a packet and, if it is a datagram, runs it.
    pub fn packet(&mut self, packet: &Packet<'_>) -> io::Result<()> {
        let p = &mut self.packets;
        if p.packets == 0 {
            p.first_ns = packet.ts_ns;
        }
        p.packets += 1;
        p.last_ns = packet.ts_ns;
        if packet.truncated() {
            p.truncated += 1;
        }
        match datagram(packet.bytes) {
            Ok(d) => {
                self.datagram(&d, packet.ts_ns)?;
            }
            Err(Skip::NotIpv4) => p.not_ipv4 += 1,
            Err(Skip::NotUdp) => p.not_udp += 1,
            Err(Skip::Fragment) => p.fragments += 1,
            Err(Skip::Short) => p.short += 1,
        }
        Ok(())
    }

    /// Runs one datagram through the pipeline and tallies the outcome.
    pub fn datagram(&mut self, d: &Datagram<'_>, recv_ns: UnixNano) -> io::Result<Outcome> {
        self.packets.datagrams += 1;
        self.packets.bytes += d.payload.len() as u64;

        let outcome = self.pipeline.ingest(d.payload, recv_ns);

        let port = self.ports.entry((d.dst, d.port)).or_default();
        port.received += 1;

        let Ok(code) = TrCode::from_message(d.payload) else {
            // Five bytes short of a trcode: nothing to tally it under.
            return Ok(outcome);
        };
        *port.codes.entry(code.as_u64()).or_insert(0) += 1;

        let tally = self.codes.entry(code.as_u64()).or_insert_with(|| CodeTally::new(code));
        tally.received += 1;
        tally.saw_length(d.payload.len());
        if d.payload.last() != Some(&END_KEYWORD) {
            tally.bad_end += 1;
        }
        match outcome {
            Outcome::Published { .. } => {
                tally.published += 1;
                if let Some(w) = self.dump.as_mut() {
                    write_tsv(w, code, &self.pipeline.sink().record)?;
                }
            }
            Outcome::FilteredTrCode => tally.filtered += 1,
            Outcome::FilteredIsin => tally.filtered_isin += 1,
            Outcome::UnknownTrCode(_) => tally.unknown += 1,
            Outcome::WrongLength { .. } => tally.wrong_length += 1,
            Outcome::Failed(e) => tally.refused(e, d.payload),
            Outcome::TooShort => {}
        }
        Ok(outcome)
    }

    /// The pipeline's own counters.
    #[inline]
    pub fn stats(&self) -> &Stats {
        self.pipeline.stats()
    }

    /// Counts over the whole capture.
    #[inline]
    pub const fn packets(&self) -> &PacketTally {
        &self.packets
    }

    /// Per-trcode tallies, most received first.
    pub fn codes(&self) -> Vec<&CodeTally> {
        let mut v: Vec<&CodeTally> = self.codes.values().collect();
        v.sort_by(|a, b| b.received.cmp(&a.received).then_with(|| a.code.as_u64().cmp(&b.code.as_u64())));
        v
    }

    /// The tally for one code, if any datagram carried it.
    pub fn code(&self, code: TrCode) -> Option<&CodeTally> {
        self.codes.get(&code.as_u64())
    }

    /// Per-group-and-port tallies, in address order.
    #[inline]
    pub const fn ports(&self) -> &BTreeMap<([u8; 4], u16), PortTally> {
        &self.ports
    }

    /// Flushes the dump.
    pub fn finish(mut self) -> io::Result<()> {
        if let Some(w) = self.dump.as_mut() {
            w.flush()?;
        }
        Ok(())
    }

    /// Writes the report: the capture, every trcode, every refusal kept, and
    /// what each port carried.
    pub fn write_report(&self, w: &mut impl Write) -> io::Result<()> {
        let p = &self.packets;
        let s = self.stats();
        writeln!(
            w,
            "capture: {} packets · {} datagrams · {} bytes · {} → {}",
            p.packets,
            p.datagrams,
            p.bytes,
            Timestamp(p.first_ns),
            Timestamp(p.last_ns)
        )?;
        writeln!(
            w,
            "skipped: not-ipv4 {} · not-udp {} · fragment {} · short {} · truncated {}",
            p.not_ipv4, p.not_udp, p.fragments, p.short, p.truncated
        )?;
        writeln!(
            w,
            "pipeline: recv {} · pub {} · filtered {} (trcode {} isin {}) · dropped {} \
             (short {} unknown {} len {} fail {})",
            s.received,
            s.published,
            s.filtered(),
            s.filtered_trcode,
            s.filtered_isin,
            s.dropped(),
            s.too_short,
            s.unknown_trcode,
            s.wrong_length,
            s.decode_failed,
        )?;

        writeln!(w)?;
        writeln!(
            w,
            "{:<7} {:>11} {:<20} {:>8} {:>11} {:>11} {:>9} {:>8} {:>8} {:>9}",
            "trcode", "recv", "len", "end!=FF", "published", "filtered", "isin-out", "unknown", "wronglen", "fail"
        )?;
        for t in self.codes() {
            let mut lengths = t.lengths.clone();
            lengths.sort_by_key(|&(_, n)| core::cmp::Reverse(n));
            let mut len = String::new();
            for (i, (l, n)) in lengths.iter().enumerate() {
                if i > 0 {
                    len.push(' ');
                }
                if lengths.len() == 1 {
                    len.push_str(&l.to_string());
                } else {
                    len.push_str(&format!("{l}×{n}"));
                }
            }
            writeln!(
                w,
                "{:<7} {:>11} {:<20} {:>8} {:>11} {:>11} {:>9} {:>8} {:>8} {:>9}",
                t.code,
                t.received,
                len,
                t.bad_end,
                t.published,
                t.filtered,
                t.filtered_isin,
                t.unknown,
                t.wrong_length,
                t.failed,
            )?;
        }

        let mut any = false;
        for t in self.codes() {
            for f in &t.failures {
                if !any {
                    writeln!(w)?;
                    writeln!(w, "refused:")?;
                    any = true;
                }
                writeln!(w, "{} ×{}: {} · {}", t.code, f.count, f.error, Head(f.head()))?;
            }
        }

        writeln!(w)?;
        writeln!(w, "ports:")?;
        for ((dst, port), t) in &self.ports {
            let mut codes: Vec<(&u64, &u64)> = t.codes.iter().collect();
            codes.sort_by(|a, b| b.1.cmp(a.1));
            let mut list = String::new();
            for (k, n) in codes {
                if !list.is_empty() {
                    list.push(' ');
                }
                list.push_str(&format!("{}:{n}", TrCode::from_u64(*k)));
            }
            writeln!(
                w,
                "{}.{}.{}.{}:{:<5} {:>11}  {list}",
                dst[0], dst[1], dst[2], dst[3], port, t.received
            )?;
        }
        Ok(())
    }
}

/// Bytes printed as ASCII, anything else as `.`.
struct Head<'a>(&'a [u8]);

impl fmt::Display for Head<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("\"")?;
        for &b in self.0 {
            let c = if (0x20..0x7f).contains(&b) { b as char } else { '.' };
            write!(f, "{c}")?;
        }
        f.write_str("\"")
    }
}

// ===========================================================================
// The dump
// ===========================================================================

/// The kind-specific tail of a TSV line, after the common columns.
///
/// Books are written ten levels deep whatever the record's depth, so a line's
/// shape depends only on its kind and a reader can index columns.
const TSV_HEADER: &str = "trcode\tkind\tsymbol\tvenue_ns\trecv_ns\tflags\tdepth\tprice_scale\tqty_scale\t…";

/// Writes the header line of a dump.
pub fn write_tsv_header(w: &mut impl Write) -> io::Result<()> {
    writeln!(w, "{TSV_HEADER}")
}

/// Writes one published record as one line.
///
/// Common columns: `trcode kind symbol venue_ns recv_ns flags depth
/// price_scale qty_scale`. Then, by kind:
///
/// | kind | columns |
/// |---|---|
/// | `Quote` | `quote_ext_kind quote_ext level_flags` then `bid_price bid_qty bid_count` ×10 then `ask_…` ×10 |
/// | `Trade` | `price qty trade_kind trade_flags cumulative_qty dyn_upper dyn_lower trade_yield` |
/// | `TradeQuote` | the `Trade` columns, then the `Quote` columns |
/// | `PriceLimit` | `applied_tod upper lower sequence board category upper_stage lower_stage` |
/// | `DynamicPriceLimit` | `applied_tod upper lower sequence board category action` |
/// | `MarketSchedule` | `event_tod expected_tod board_event_group category product board event action session halt_reason halt_type step direction flags` |
///
/// Prices and sizes are the wire integers; the scales say where the point is.
pub fn write_tsv(w: &mut impl Write, code: TrCode, rec: &WireRecord) -> io::Result<()> {
    let h = &rec.header;
    let kind = rec.kind().ok();
    write!(
        w,
        "{code}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        kind.map_or("?", kind_name),
        String::from_utf8_lossy(h.symbol_bytes()),
        h.venue_ns,
        h.recv_ns,
        h.flags,
        h.depth,
        h.price_scale,
        h.qty_scale,
    )?;
    match kind {
        Some(WireKind::Quote) => {
            if let Ok(q) = rec.quote() {
                quote_columns(w, q)?;
            }
        }
        Some(WireKind::Trade) => {
            if let Ok(t) = rec.trade() {
                trade_columns(w, t)?;
            }
        }
        Some(WireKind::TradeQuote) => {
            if let Ok(tq) = rec.trade_quote() {
                trade_columns(w, &tq.trade)?;
                quote_columns(w, &tq.quote)?;
            }
        }
        Some(WireKind::PriceLimit) => {
            if let Ok(p) = rec.price_limit() {
                write!(
                    w,
                    "\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    p.applied_time_of_day,
                    p.upper_price,
                    p.lower_price,
                    p.sequence,
                    String::from_utf8_lossy(&p.board_id),
                    String::from_utf8_lossy(&p.information_category),
                    p.upper_stage,
                    p.lower_stage,
                )?;
            }
        }
        Some(WireKind::DynamicPriceLimit) => {
            if let Ok(p) = rec.dynamic_price_limit() {
                write!(
                    w,
                    "\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    p.applied_time_of_day,
                    p.upper_price,
                    p.lower_price,
                    p.sequence,
                    String::from_utf8_lossy(&p.board_id),
                    String::from_utf8_lossy(&p.information_category),
                    p.action,
                )?;
            }
        }
        Some(WireKind::MarketSchedule) => {
            if let Ok(m) = rec.market_schedule() {
                write!(
                    w,
                    "\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    m.event_time_of_day,
                    m.expected_time_of_day,
                    m.board_event_group,
                    String::from_utf8_lossy(&m.information_category),
                    String::from_utf8_lossy(&m.product_id),
                    String::from_utf8_lossy(&m.board_id),
                    String::from_utf8_lossy(&m.board_event_id),
                    String::from_utf8_lossy(&m.session_action),
                    String::from_utf8_lossy(&m.session_id),
                    String::from_utf8_lossy(&m.halt_reason),
                    m.halt_type,
                    m.step,
                    m.expansion_direction,
                    m.schedule_flags,
                )?;
            }
        }
        _ => {}
    }
    writeln!(w)
}

fn quote_columns(w: &mut impl Write, q: &QuotePayload) -> io::Result<()> {
    write!(w, "\t{}\t{}\t{}", q.quote_ext_kind, q.quote_ext, q.level_flags)?;
    for l in &q.bid {
        write!(w, "\t{}\t{}\t{}", l.price, l.qty, l.order_count)?;
    }
    for l in &q.ask {
        write!(w, "\t{}\t{}\t{}", l.price, l.qty, l.order_count)?;
    }
    Ok(())
}

fn trade_columns(w: &mut impl Write, t: &TradePayload) -> io::Result<()> {
    write!(
        w,
        "\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        t.price,
        t.qty,
        t.trade_kind,
        t.trade_flags,
        t.cumulative_qty,
        t.dyn_upper,
        t.dyn_lower,
        t.trade_yield,
    )
}

const fn kind_name(k: WireKind) -> &'static str {
    match k {
        WireKind::Quote => "Quote",
        WireKind::Trade => "Trade",
        WireKind::TradeQuote => "TradeQuote",
        WireKind::OpenInterest => "OpenInterest",
        WireKind::InvestorStats => "InvestorStats",
        WireKind::SnapshotDelta => "SnapshotDelta",
        WireKind::PriceLimit => "PriceLimit",
        WireKind::MarketSchedule => "MarketSchedule",
        WireKind::Heartbeat => "Heartbeat",
        WireKind::DynamicPriceLimit => "DynamicPriceLimit",
    }
}

const _: () = assert!(TRCODE_LEN == 5);
