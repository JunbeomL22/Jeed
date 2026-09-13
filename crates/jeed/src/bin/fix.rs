//! `jeed-fix conf/fix.toml [--check] [--no-pin]`
//!
//! Exit codes: 0 stopped on request · 1 usage or conf · 2 could not start ·
//! 3 a feed died.

use core::sync::atomic::AtomicBool;
use jeed::conf::FixConf;
use jeed::conf::fix::venue_name_or;
use jeed::cpu::Topology;
use jeed::fix::{self, Options};
use jeed::{boot_id, error, feed, info, signal, warn};
use jeed_fix::recv::{Mode, SubscriptionType};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

const USAGE: &str = "usage: jeed-fix <conf.toml> [--check] [--no-pin]\n  \
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
    let conf = match FixConf::load(&args.conf) {
        Ok(c) => c,
        Err(e) => {
            error!("{}: {e}", args.conf.display());
            return ExitCode::from(1);
        }
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
    for w in conf.warnings(topology.as_ref()) {
        warn!("{w}");
    }

    if let Some(t) = &topology {
        info!("{} logical processors", t.logical());
    }
    for feed in &conf.feeds {
        info!(
            "feed {} · {} · {} · cores {:?} · ring {} ({} slots) · {}:{} · {}→{} · {}/{} decimals · {} · {} symbol(s)",
            feed.name,
            venue_name_or(feed.venue),
            match feed.mode { Mode::Spin => "spin", Mode::Block => "block" },
            feed.cores,
            feed.ring,
            feed.ring_slots,
            feed.host,
            feed.port,
            feed.sender_comp_id,
            feed.target_comp_id,
            feed.price_decimals,
            feed.qty_decimals,
            match feed.subscription {
                SubscriptionType::Snapshot => "snapshot once",
                SubscriptionType::SnapshotPlusUpdates => "snapshot then updates",
                SubscriptionType::Unsubscribe => "unsubscribe",
            },
            feed.symbols.len(),
        );
        info!("  {}", feed.symbols.join(", "));
    }
    info!(
        "health · heartbeat {} ms · stale {} ms · report every {} s",
        conf.health.heartbeat_ns / 1_000_000,
        conf.health.stale_ns / 1_000_000,
        conf.report_secs
    );

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
    let feeds = match fix::start(&conf, opts, Arc::clone(&stop)) {
        Ok(f) => f,
        Err(e) => {
            error!("could not start: {e}");
            return ExitCode::from(2);
        }
    };

    // ── run ─────────────────────────────────────────────────────────────
    if feed::wait(feeds, &stop) { ExitCode::from(3) } else { ExitCode::SUCCESS }
}
