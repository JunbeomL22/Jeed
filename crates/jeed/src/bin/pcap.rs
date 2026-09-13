//! `jeed-pcap <capture.pcap> [--table conf/krx_trcodes.toml] [--trcodes A,B]
//! [--isin X,Y] [--dump records.tsv] [--report report.txt] [--limit N]`
//!
//! Runs every UDP datagram of a capture through the KRX pipeline and reports
//! what each trcode turned into. Nothing is created: no ring, no socket.
//!
//! Exit codes: 0 done · 1 usage or conf · 2 the capture could not be read.

use jeed::conf::TrCodeTable;
use jeed::pcap::{PcapError, Reader, Replay};
use jeed::{error, info, warn};
use jeed_krx::TrCode;
use jeed_krx::recv::{IsinFilter, TrCodeFilter};
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

const USAGE: &str = "usage: jeed-pcap <capture.pcap> [options]\n  \
    --table <path>     trcode table; the allow-set is every code it names\n                     \
    (default conf/krx_trcodes.toml)\n  \
    --trcodes A,B,C    allow-set, instead of the table\n  \
    --isin X,Y         ISIN allow-set (default: every instrument)\n  \
    --dump <file.tsv>  write every published record as one line\n  \
    --report <file>    write the report there instead of stdout\n  \
    --limit N          stop after N packets";

/// Progress line every this many packets.
const PROGRESS_EVERY: u64 = 5_000_000;

struct Args {
    capture: PathBuf,
    table: PathBuf,
    trcodes: Option<Vec<TrCode>>,
    isins: Option<Vec<[u8; 12]>>,
    dump: Option<PathBuf>,
    report: Option<PathBuf>,
    limit: Option<u64>,
}

fn parse_args() -> Result<Args, String> {
    let mut capture = None;
    let mut table = PathBuf::from("conf/krx_trcodes.toml");
    let mut trcodes = None;
    let mut isins = None;
    let mut dump = None;
    let mut report = None;
    let mut limit = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value\n{USAGE}"));
        match arg.as_str() {
            "--table" => table = PathBuf::from(value("--table")?),
            "--trcodes" => {
                let list = value("--trcodes")?;
                let mut codes = Vec::new();
                for item in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    let bytes: [u8; 5] = item
                        .as_bytes()
                        .try_into()
                        .map_err(|_| format!("`{item}` is not a five-byte trcode"))?;
                    codes.push(TrCode::new(bytes));
                }
                trcodes = Some(codes);
            }
            "--isin" => {
                let list = value("--isin")?;
                let mut out = Vec::new();
                for item in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    let bytes: [u8; 12] = item
                        .as_bytes()
                        .try_into()
                        .map_err(|_| format!("`{item}` is not a twelve-byte ISIN"))?;
                    out.push(bytes);
                }
                isins = Some(out);
            }
            "--dump" => dump = Some(PathBuf::from(value("--dump")?)),
            "--report" => report = Some(PathBuf::from(value("--report")?)),
            "--limit" => {
                let v = value("--limit")?;
                limit = Some(v.parse().map_err(|_| format!("`{v}` is not a packet count"))?);
            }
            "-h" | "--help" => return Err(USAGE.to_owned()),
            s if s.starts_with('-') => return Err(format!("unknown option `{s}`\n{USAGE}")),
            s => {
                if capture.replace(PathBuf::from(s)).is_some() {
                    return Err(format!("more than one capture given\n{USAGE}"));
                }
            }
        }
    }
    let capture = capture.ok_or_else(|| USAGE.to_owned())?;
    Ok(Args { capture, table, trcodes, isins, dump, report, limit })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(1);
        }
    };

    // ── the allow-sets ──────────────────────────────────────────────────
    let trcodes = match &args.trcodes {
        Some(codes) => TrCodeFilter::new(codes.iter().copied()),
        None => match TrCodeTable::load(&args.table) {
            Ok(t) => {
                info!("{}: {} codes (spec {} / channel {})", args.table.display(), t.len(), t.spec_version, t.channel_spec_version);
                TrCodeFilter::new(t.codes())
            }
            Err(e) => {
                error!("{}: {e}", args.table.display());
                return ExitCode::from(1);
            }
        },
    };
    let isins = match &args.isins {
        Some(list) => IsinFilter::new(list.iter().copied()),
        None => IsinFilter::all(),
    };
    info!("allow-set: {} trcodes · {}", trcodes.len(), match isins.len() {
        0 => "every instrument".to_owned(),
        n => format!("{n} instruments"),
    });

    // ── the files ───────────────────────────────────────────────────────
    let dump = match &args.dump {
        Some(path) => match File::create(path) {
            Ok(f) => Some(BufWriter::with_capacity(1 << 20, f)),
            Err(e) => {
                error!("{}: {e}", path.display());
                return ExitCode::from(1);
            }
        },
        None => None,
    };
    let mut reader = match Reader::open(&args.capture) {
        Ok(r) => r,
        Err(e) => {
            error!("{}: {e}", args.capture.display());
            return ExitCode::from(2);
        }
    };
    let mut replay = match Replay::new(trcodes, isins, dump) {
        Ok(r) => r,
        Err(e) => {
            error!("dump: {e}");
            return ExitCode::from(1);
        }
    };

    // ── the replay ──────────────────────────────────────────────────────
    let started = Instant::now();
    let mut cut_short = None;
    loop {
        if args.limit.is_some_and(|n| reader.packets() >= n) {
            break;
        }
        let packet = match reader.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(e @ PcapError::Truncated { .. }) => {
                // A tap that was killed rather than stopped. Everything before
                // the cut is a whole packet and has been replayed.
                cut_short = Some(e);
                break;
            }
            Err(e) => {
                error!("{}: {e}", args.capture.display());
                return ExitCode::from(2);
            }
        };
        if let Err(e) = replay.packet(&packet) {
            error!("dump: {e}");
            return ExitCode::from(1);
        }
        if reader.packets().is_multiple_of(PROGRESS_EVERY) {
            progress(&replay, started);
        }
    }
    progress(&replay, started);
    if let Some(e) = cut_short {
        warn!("{}: {e}", args.capture.display());
    }

    // ── the report ──────────────────────────────────────────────────────
    let written = match &args.report {
        Some(path) => File::create(path)
            .and_then(|f| {
                let mut w = BufWriter::new(f);
                replay.write_report(&mut w)?;
                w.flush()
            })
            .map_err(|e| format!("{}: {e}", path.display())),
        None => {
            let stdout = io::stdout();
            let mut w = stdout.lock();
            replay.write_report(&mut w).and_then(|()| w.flush()).map_err(|e| format!("stdout: {e}"))
        }
    };
    if let Err(msg) = written {
        error!("{msg}");
        return ExitCode::from(1);
    }
    if let Err(e) = replay.finish() {
        error!("dump: {e}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn progress<W: Write>(replay: &Replay<W>, started: Instant) {
    let p = replay.packets();
    let s = replay.stats();
    let secs = started.elapsed().as_secs_f64();
    info!(
        "{} packets · {} datagrams · {:.1} GB · pub {} · fail {} · {:.0} s",
        p.packets,
        p.datagrams,
        p.bytes as f64 / 1e9,
        s.published,
        s.decode_failed,
        secs
    );
}
