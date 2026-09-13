//! Decodes a real `apps/smbs_to_fix` capture and checks it against the index
//! the generator wrote beside it.
//!
//! This is the test that matters most: the encoder in fractal-engine's
//! `apps/smbs_to_fix` and the decoder here are independent implementations
//! (`documents/feed_handler.md` §12), so agreement on 57,221 messages —
//! offsets, lengths, sequence numbers, message types, and venue timestamps —
//! is real evidence rather than a tautology.
//!
//! It stops at [`MdMessage`](jeed_fix::MdMessage). What SMBS turns one into
//! is the venue adapter's test, and that adapter is not in this crate
//! (`documents/todo.md` §6).
//!
//! The capture lives outside the repository, so the test reports and returns
//! when it is absent.
//!
//! ## Finding it from either side of WSL
//!
//! The same drive is `E:/Data` to Windows and `/mnt/e/Data` to WSL, so both are
//! tried in order. No `cfg` is needed: the path that is not this platform's
//! simply does not exist, and `is_file` says so. `JEED_SMBS_CAPTURE` overrides
//! both, for a capture kept somewhere else entirely.

use super::SCALES;
use jeed_fix::{
    FixError, FrameBuffer, MdEntryType, MsgType, SeqVerdict, SessionState, frame, msg_type,
    parse_admin_message, parse_md_message,
};
use std::path::{Path, PathBuf};

/// Roots the capture is looked for under, in order — the same drive as
/// Windows sees it and as WSL mounts it.
const CAPTURE_ROOTS: [&str; 2] = ["E:/Data/smbs_fix_db", "/mnt/e/Data/smbs_fix_db"];

/// Environment variable that replaces [`CAPTURE_ROOTS`] outright.
const CAPTURE_ENV: &str = "JEED_SMBS_CAPTURE";

/// Days the test will use, first one present wins.
const DAYS: [&str; 3] = ["20260202", "20260203", "20260610"];

/// One row of `usdkrw_smbs.idx.csv`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IndexRow {
    offset: usize,
    len: usize,
    msg_seq: u64,
    msg_type: u8,
    venue_ns: u64,
    local_ns: u64,
}

/// Where to look, in order.
fn roots() -> Vec<String> {
    match std::env::var(CAPTURE_ENV) {
        Ok(root) if !root.trim().is_empty() => vec![root.trim().to_owned()],
        _ => CAPTURE_ROOTS.iter().map(|&r| r.to_owned()).collect(),
    }
}

fn find_day() -> Option<PathBuf> {
    roots().iter().flat_map(|root| DAYS.iter().map(|d| Path::new(root).join(d))).find(|p| {
        p.join("usdkrw_smbs.fix").is_file() && p.join("usdkrw_smbs.idx.csv").is_file()
    })
}

/// What to say when there is nothing to replay. Naming the paths that were
/// tried is the difference between "skipped" and "skipped, and here is why".
fn no_capture() {
    eprintln!("skipping: no capture under {} (override with {CAPTURE_ENV})", roots().join(", "));
}

fn read_index(path: &Path) -> Vec<IndexRow> {
    let text = std::fs::read_to_string(path).expect("index readable");
    let mut rows = Vec::with_capacity(64 * 1024);
    for line in text.lines().skip(1) {
        if line.is_empty() {
            continue;
        }
        let mut f = line.split(',');
        let mut next = || f.next().expect("index column").trim();
        rows.push(IndexRow {
            offset: next().parse().expect("offset"),
            len: next().parse().expect("len"),
            msg_seq: next().parse().expect("msg_seq"),
            msg_type: next().as_bytes()[0],
            venue_ns: next().parse().expect("venue_unix_nano"),
            local_ns: next().parse().expect("local_unix_nano"),
        });
    }
    rows
}

#[test]
fn the_generated_capture_decodes_exactly_as_indexed() {
    let Some(day) = find_day() else {
        no_capture();
        return;
    };
    let bytes = std::fs::read(day.join("usdkrw_smbs.fix")).expect("capture readable");
    let index = read_index(&day.join("usdkrw_smbs.idx.csv"));
    assert!(!index.is_empty(), "the index must describe the capture");

    let mut session = SessionState::new();
    session.heartbeat_interval_ns = 30_000_000_000;

    let mut offset = 0usize;
    let mut counts = [0u32; 3]; // W, X, heartbeat
    let mut one_sided = 0u32;
    let mut widest_book = 0usize;

    for (i, row) in index.iter().enumerate() {
        assert_eq!(offset, row.offset, "message {i} does not start where the index says");
        let f = match frame(&bytes[offset..]) {
            Ok(f) => f,
            Err(e) => panic!("message {i} at {offset} failed to frame: {e}"),
        };
        assert_eq!(f.len(), row.len, "message {i} length");
        assert_eq!(f.begin_string, b"FIX.4.4");
        offset += f.len();

        let kind = msg_type(&f).expect("every message has 35=");
        assert_eq!(
            kind,
            MsgType::from_value(&[row.msg_type]),
            "message {i} type disagrees with the index"
        );

        match kind {
            MsgType::MarketDataSnapshot | MsgType::MarketDataIncremental => {
                let msg = parse_md_message(&f, SCALES)
                    .unwrap_or_else(|e| panic!("message {i} ({kind:?}) failed to decode: {e}"));
                assert_eq!(msg.msg_seq_num, row.msg_seq, "message {i} sequence");
                assert_eq!(
                    msg.venue_time(),
                    Some(row.venue_ns),
                    "message {i} venue time must come from 272/273, not 52"
                );
                assert_eq!(
                    msg.sending_time, row.local_ns,
                    "message {i} SendingTime is the counterparty clock"
                );
                assert_eq!(msg.declared_entries, msg.len() as u32);

                if msg.is_snapshot() {
                    counts[0] += 1;
                    let bids = msg.side_depth(MdEntryType::Bid);
                    let asks = msg.side_depth(MdEntryType::Offer);
                    widest_book = widest_book.max(bids.max(asks));
                    if bids == 0 || asks == 0 {
                        one_sided += 1;
                    }
                } else {
                    counts[1] += 1;
                }

                let verdict = session.observe(msg.msg_seq_num, msg.poss_dup, row.local_ns);
                assert_eq!(verdict, SeqVerdict::InOrder, "message {i} broke the sequence");
            }
            MsgType::Heartbeat => {
                counts[2] += 1;
                let msg = parse_admin_message(&f).expect("heartbeat decodes");
                assert_eq!(msg.msg_seq_num, row.msg_seq);
                assert_eq!(msg.sending_time, row.local_ns);
                let verdict = session.observe(msg.msg_seq_num, msg.poss_dup, row.local_ns);
                assert_eq!(verdict, SeqVerdict::InOrder, "message {i} broke the sequence");
            }
            other => panic!("message {i} has an unexpected type {other:?}"),
        }
    }

    assert_eq!(offset, bytes.len(), "the capture has trailing bytes the index does not describe");
    assert_eq!(
        frame(&bytes[offset..]),
        Err(FixError::Incomplete),
        "an empty tail asks for more bytes"
    );
    assert_eq!(session.observed as usize, index.len());
    assert_eq!((session.gaps, session.lost, session.duplicates, session.regressions), (0, 0, 0, 0));
    assert_eq!(counts[0] + counts[1] + counts[2], index.len() as u32);
    assert!(counts[0] > 0 && counts[1] > 0 && counts[2] > 0, "all three shapes exercised");
    assert_eq!(widest_book, 1, "SMBS publishes only the best bid and offer");

    eprintln!(
        "{}: {} messages ({} W, {} X, {} HB), {} one-sided books",
        day.file_name().unwrap().to_string_lossy(),
        index.len(),
        counts[0],
        counts[1],
        counts[2],
        one_sided,
    );
}

#[test]
fn the_capture_streams_through_a_fixed_receive_buffer() {
    let Some(day) = find_day() else {
        no_capture();
        return;
    };
    let bytes = std::fs::read(day.join("usdkrw_smbs.fix")).expect("capture readable");
    let index = read_index(&day.join("usdkrw_smbs.idx.csv"));

    // 1500 bytes is one ethernet frame: message boundaries land wherever
    // they land, which is the condition the live handler runs in.
    let mut buf: FrameBuffer<4096> = FrameBuffer::new();
    let mut seen = 0usize;
    let mut cursor = 0usize;
    while cursor < bytes.len() || !buf.filled().is_empty() {
        if cursor < bytes.len() {
            let end = (cursor + 1500).min(bytes.len());
            let written = buf.push(&bytes[cursor..end]);
            cursor += written;
            if written == 0 && buf.peek().expect("framing").is_none() {
                panic!("buffer full without a complete message");
            }
        }
        let mut progressed = false;
        while let Some(f) = buf.peek().expect("framing") {
            assert_eq!(f.len(), index[seen].len, "message {seen} length");
            let len = f.len();
            buf.consume(len);
            seen += 1;
            progressed = true;
        }
        if !progressed && cursor >= bytes.len() {
            break;
        }
    }
    assert_eq!(seen, index.len(), "every message framed across arbitrary read boundaries");
    assert!(buf.filled().is_empty());
}
