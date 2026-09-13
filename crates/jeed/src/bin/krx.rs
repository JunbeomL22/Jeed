//! `jeed-krx conf/krx.toml [--check] [--no-pin]`
//!
//! Exit codes: 0 stopped on request · 1 usage or conf · 2 could not start ·
//! 3 a feed died.

use core::sync::atomic::{AtomicBool, Ordering};
use jeed::conf::{KrxConf, TrCodeTable};
use jeed::cpu::Topology;
use jeed::krx::{self, Options};
use jeed::{boot_id, error, info, signal, warn};
use jeed_krx::recv::{IsinFilter, Mode, parse_isin_list};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

const USAGE: &str = "usage: jeed-krx <conf.toml> [--check] [--no-pin]\n  \
    --check   validate the conf and exit without creating anything\n  \
    --no-pin  do not pin feed threads (a development box)";

struct Args {
    conf: PathBuf,
    check: bool,
    pin: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut conf = None;
    let mut check = false;
    let mut pin = true;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--check" => check = true,
            "--no-pin" => pin = false,
            "-h" | "--help" => return Err(USAGE.to_owned()),
            s if s.starts_with('-') => return Err(format!("unknown option `{s}`\n{USAGE}")),
            s => {
                if conf.replace(PathBuf::from(s)).is_some() {
                    return Err(format!("more than one conf given\n{USAGE}"));
                }
            }
        }
    }
    let conf = conf.ok_or_else(|| USAGE.to_owned())?;
    Ok(Args { conf, check, pin })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(1);
        }
    };

    // ── conf ────────────────────────────────────────────────────────────
    let conf = match KrxConf::load(&args.conf) {
        Ok(c) => c,
        Err(e) => {
            error!("{}: {e}", args.conf.display());
            return ExitCode::from(1);
        }
    };

    let table = match &conf.trcode_table {
        None => None,
        Some(path) => match TrCodeTable::load(path) {
            Ok(t) => {
                info!("trcode table {} · spec {} / channels {} · {} codes",
                    path.display(), t.spec_version, t.channel_spec_version, t.len());
                Some(t)
            }
            Err(e) => {
                error!("{e}");
                return ExitCode::from(1);
            }
        },
    };

    let topology = match Topology::detect() {
        Ok(t) => Some(t),
        Err(e) => {
            warn!("SMT topology: {e}");
            None
        }
    };

    if let Err(e) = conf.validate(topology.as_ref()) {
        error!("{}: {e}", args.conf.display());
        return ExitCode::from(1);
    }
    for w in conf.warnings(table.as_ref(), topology.as_ref()) {
        warn!("{w}");
    }

    if let Some(t) = &topology {
        info!("{} logical processors", t.logical());
    }
    for feed in &conf.feeds {
        info!(
            "feed {} · {} · cores {:?} · ring {} ({} slots) · {} sockets · {} trcodes",
            feed.name,
            match feed.mode { Mode::Spin => "spin", Mode::Block => "block" },
            feed.cores,
            feed.ring,
            feed.ring_slots,
            feed.sockets.len(),
            feed.trcodes.len(),
        );
    }
    info!(
        "health · heartbeat {} ms · stale {} ms · report every {} s",
        conf.health.heartbeat_ns / 1_000_000,
        conf.health.stale_ns / 1_000_000,
        conf.report_secs
    );

    let isins = match &conf.isin_list {
        None => {
            info!("isin filter: none (every 종목 passes)");
            IsinFilter::all()
        }
        Some(path) => match std::fs::read_to_string(path) {
            Err(e) => {
                error!("{}: {e}", path.display());
                return ExitCode::from(1);
            }
            Ok(text) => match parse_isin_list(&text) {
                Ok(f) => {
                    info!("isin filter: {} 종목 from {}", f.len(), path.display());
                    f
                }
                Err(e) => {
                    error!("{}: {e}", path.display());
                    return ExitCode::from(1);
                }
            },
        },
    };

    if args.check {
        info!("conf ok");
        return ExitCode::SUCCESS;
    }

    // ── start ───────────────────────────────────────────────────────────
    if let Err(e) = signal::install() {
        error!("signal handler: {e}");
        return ExitCode::from(2);
    }

    let stop = Arc::new(AtomicBool::new(false));
    let opts = Options { boot_id: boot_id(), pin: args.pin, report_ns: conf.report_secs * 1_000_000_000 };
    let mut feeds = match krx::start(&conf, &isins, opts, Arc::clone(&stop)) {
        Ok(f) => f,
        Err(e) => {
            error!("could not start: {e}");
            return ExitCode::from(2);
        }
    };

    // ── run ─────────────────────────────────────────────────────────────
    let mut failed = false;
    loop {
        if signal::requested() && !stop.load(Ordering::Relaxed) {
            info!("stop requested");
            stop.store(true, Ordering::Relaxed);
        }

        if let Some(i) = feeds.iter().position(krx::Feed::is_finished) {
            let feed = feeds.swap_remove(i);
            let name = feed.name.clone();
            match feed.join() {
                Ok(()) => info!("{name}: stopped"),
                Err(e) => {
                    error!("{name}: {e}");
                    failed = true;
                    // One feed down is the whole handler down: the consumer
                    // sees one boot_id per ring and should not be left with
                    // half a market.
                    stop.store(true, Ordering::Relaxed);
                }
            }
        }

        if feeds.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    if failed { ExitCode::from(3) } else { ExitCode::SUCCESS }
}
