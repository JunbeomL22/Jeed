# Jeed

**J**unbeom f**eed** handler. Receives KRX UDP multicast, crypto-exchange WebSocket
streams and FIX 4.4 market data, decodes each into a **fixed-layout 640-byte wire
record**, and publishes it to a shared-memory ring. The consumer (an OMS) runs as a
**separate process** and depends on nothing but the wire format and the ring. Pure
Rust; the only external dependency is `rustls`.

```text
 KRX circuit ──UDP──▶ jeed-krx    ─┐
 crypto venues ──WSS──▶ jeed-crypto ─┼─▶ decode · filter · normalise ─▶ WireRecord ─▶ shm SPSC ring ─▶ consumer
 FIX venue ──TCP──▶ jeed-fix     ─┘        stateless · no book kept                     process boundary
```

## Why a separate process

- **A parsing bug does not redeploy the OMS.** The consumer depends on `jeed-wire` and
  `jeed-shm` only. KRX message layouts, exchange JSON dialects, TLS — all of it stays
  on this side of the ring.
- **The hot path is one busy-spin loop pinned to one core.** The ring producer never
  blocks; the consumer learns about its own losses from `producer_seq`.
- **The wire format is the lock-in, not the ring.** Bumping `WIRE_FORMAT_VERSION` is a
  deploy-both-binaries event and is not done lightly.

Design rationale: [`documents/feed_handler.md`](documents/feed_handler.md). Work log,
decisions and measurements: [`documents/todo.md`](documents/todo.md). The traps you must
know before touching the code: [`CLAUDE.md`](CLAUDE.md) (Korean).

## Crates

```text
crates/
  jeed-wire/     Wire record ABI. Zero dependencies. The only thing a consumer depends on
  jeed-convert/  Fixed-width ASCII numeric parsing (SWAR)
  jeed-shm/      Named mapping (Win32) / shm_open (POSIX) + SPSC ring (producer / consumer)
  jeed-krx/      KRX UDP reception + message decoders → WireRecord
  jeed-fix/      FIX 4.4 protocol → MdMessage. Knows no venue; the adapter is a trait
  jeed-crypto/   Crypto WebSocket JSON → WireRecord
                 binance · upbit · bithumb · okx · bybit · bitget · gate · kucoin
                 + recv/: WS·TLS receive loop, per-venue Router, one-shot HTTP client
  jeed/          The binaries: conf (own TOML reader) · core pinning · segments · signals · report
                 src/bin/krx.rs → jeed-krx   src/bin/crypto.rs → jeed-crypto
                 src/bin/fix.rs → jeed-fix   src/bin/pcap.rs   → jeed-pcap (capture replay)
```

```text
jeed-wire ←── jeed-shm ←──┬── jeed (bin)
    ↑                     │
    ├── jeed-krx ─────────┤
    ├── jeed-fix ─────────┤
    └── jeed-crypto ──────┘
    ↑
    └──────────────────── a consumer needs jeed-wire + jeed-shm, nothing else
```

The handler crates (`jeed-krx`, `jeed-fix`, `jeed-crypto`) **do not depend on `jeed-shm`**.
They write into a `jeed_wire::RecordSink` and the binary connects that sink to a ring, so
every decoder is tested on captured bytes with no socket and no shared memory in sight.

## The wire record

`WireRecord` is 640 bytes, `#[repr(C)]`, cache-line aligned, with no implicit padding
(asserted at compile time).

```text
RecordHeader 64 B   kind · venue · symbol[24] · recv_ns · venue_ns · producer_seq · price/qty scale · depth · flags
payload     544 B   Quote (10 levels a side) · Trade · TradeQuote (print + book, atomic) · SnapshotDelta (32 changed levels)
                    PriceLimit · DynamicPriceLimit · MarketSchedule · Heartbeat · …
tail         32 B   explicit padding
```

- **Conclusions travel, evidence does not.** The `STALE` flag is on the wire; a venue-specific
  sequence-gap verdict is not — a bit the consumer must branch on `venue` to read is not a
  common field.
- **The identifier is the venue's own `(venue, symbol)`.** Process-local interned ids do not
  cross the boundary. Binance spot and USD-M both list `BTCUSDT`, so they are different venue
  bytes.
- **Scale is a property of the header, not of a value.** Prices are integers; the number of
  decimals is stamped once in the header.
- **Deltas are dropped, never truncated.** A snapshot cut to ten levels is a shallow book;
  a delta cut to thirty-two is a wrong book forever.

## The shm ring

- **Overwrite, not back-pressure.** When full, the oldest slot is overwritten. The producer
  never blocks.
- **`producer_seq` in the record is the slot's seqlock.** `u64::MAX` while a slot is being
  written, then the body, a release fence, then the real sequence. A consumer reads the
  sequence before and after to reject a torn read.
- **A consumer attaches at the live edge.** It never replays a stale lap.
- **`boot_id`** identifies a producer restart; heartbeat records separate "quiet market" from
  "dead producer".

```rust
use jeed_shm::{Recv, RingConsumer, SegmentName};
use jeed_wire::WireRecord;

let name = SegmentName::local("jeed.krx.hot")?;
let mut rx = RingConsumer::attach(&name)?;
let mut rec = WireRecord::zeroed();
loop {
    match rx.try_recv(&mut rec) {
        Recv::Record => { /* rec.kind(), rec.header.venue(), rec.quote() … */ }
        Recv::Lagged(n) => { /* n records were overwritten — resync if you follow a delta chain */ }
        Recv::Restarted { .. } => { /* the producer rebooted */ }
        Recv::Empty => { /* spin or yield */ }
    }
}
```

## Examples

Each one runs on its own — the datagrams and JSON frames are embedded, so no circuit,
exchange connection or running producer is needed.

```bash
cargo run -p jeed        --example krx         # examples/krx.rs — the whole KRX pipeline in one process:
                                               # conf → ring → socket → feed thread → a datagram on loopback
                                               # multicast → the record a consumer reads off the ring
cargo run -p jeed-shm    --example roundtrip   # producer + consumer in one process:
                                               # publish, read back, Lagged, Restarted
cargo run -p jeed-krx    --example decode      # B601F book + A301F print through Pipeline::ingest,
                                               # plus a filtered code, a wrong length and a refused field
cargo run -p jeed-crypto --example decode      # Binance spot trade + depth, Upbit trade (exponent price),
                                               # a wrong-symbol frame and an over-precise price, both refused
cargo run -p jeed-shm    --example consumer -- jeed.krx.hot     # tail a live ring (start a producer first)
```

`consumer` is the consumer side as an OMS would write it: `jeed-wire` + `jeed-shm`, attach at the
live edge, one line per record, and the two things a consumer must handle — `Lagged(n)` and
`Restarted`.

## Building and running

```bash
cargo build --release
./target/release/jeed-krx    conf/krx.toml    --check    # validate only, create nothing
./target/release/jeed-crypto conf/crypto.toml --no-pin   # run without core pinning (a dev box)
./target/release/jeed-fix    conf/fix.toml
```

Every binary takes `<conf.toml> [--check] [--no-pin]`. Paths in a conf are relative to the
working directory, so start from the repository root. Exit codes: `0` stopped on request
(Ctrl-C / SIGTERM) · `1` usage or conf error · `2` could not start · `3` a feed died. **If one
feed dies, all feeds stop** — the consumer is never left looking at half a market.

The conf rules are the same for all three:

- **An unknown key is a start-up failure.** `ring_slot = 16384` next to a default `ring_slots`
  must not run silently with the wrong size.
- **An absent guard is an enabled guard.** No `[health]` section means a 100 ms heartbeat
  record and a 500 ms stale threshold. To turn one off, write `0`.
- **Placement is checked before anything is created**: duplicate cores, a spinning feed on
  more than one core, a spinning core's SMT sibling in use by another feed, a ring size that
  is not a power of two, duplicate ring names. `--check` runs exactly these checks and exits.
- **Everything that can fail, fails before a thread starts.** Rings are created, sockets
  joined and routers built on the main thread in conf order. A mistake in the second feed
  ends the process with nothing running.

Every feed thread writes one report line per `report_secs` (default 10) with its counters.
Refused messages are logged with their cause and the first bytes — the first five in full,
then one in a thousand — which is how most of the venue surprises below were found.

### `jeed-krx` — KRX UDP multicast

One `[[feed]]` is a set of multicast sockets, one receive thread, one ring. A single port
carries **every data class of a product group** (`B6` book, `G7` print+book, `A3` print,
`V1`/`Q2` price limits, and a dozen others), so `sockets` (what to join) and `trcodes` (what
to keep) are separate lists. The five-byte trcode (`B604F` = data class `B6`, product group
`04F`) is the dispatch key; anything not in `trcodes` is dropped before a ring slot is claimed.

```toml
trcode_table = "conf/krx_trcodes.toml"   # generated from the KRX standards (tools/)

[[feed]]
name  = "hot"
mode  = "spin"            # busy-spin, 100 % of one core
cores = [2]               # exactly one core for a spinning feed; leave its SMT sibling empty
ring  = "jeed.krx.hot"
ring_slots = 16384        # × 640 B = 10 MiB; must be a power of two,
                          # and must stay inside one CCD's L3 (32 MiB)
sockets = ["233.38.231.92:10302", "233.38.231.92:10304"]     # from your circuit assignment
trcodes = ["B601F", "G701F", "B604F", "G704F", "V101F", "Q201F"]

[[feed]]
name  = "cold"
mode  = "block"           # poll/WSAPoll, ~0 % CPU
cores = [4, 5, 6, 7]
ring  = "jeed.krx.cold"
ring_slots = 4096
sockets = ["233.38.231.93:10315"]
trcodes = ["M401F", "M403F"]

# isin_list = "conf/isins.txt"   # optional per-instrument allow-list (one 종목코드 per line)
[health]
heartbeat_ms = 100
stale_ms = 500
```

What the decoders cover today: derivatives books (`B6`, 5- and 10-deep), prints (`A3`),
print+book (`G7`), price-limit expansion (`V1`), dynamic price bands (`Q2`), stocks (`B6`
590 B, `A3`), ETF/ETN/ELW LP books (`B7` 830 B), market schedules (`M4`). Bonds have decoders
but the live circuit's bond layout does not match them yet (see `documents/todo.md` §16).

The socket ↔ trcode mapping is a circuit fact the conf cannot verify. Two things help: the
report line warns about any configured trcode that has **never arrived on any socket**, and
`jeed-pcap` (below) prints, per multicast port, exactly which trcodes a capture carried.

### `jeed-pcap` — replay a capture through the KRX pipeline

```bash
./target/release/jeed-pcap E:/Data/krx_pcap/20260807.pcap --report report.txt
./target/release/jeed-pcap capture.pcap --trcodes B604F,G704F --isin KR4A50680003 --dump records.tsv
```

Reads a libpcap file (Ethernet · IPv4 · UDP, VLAN tags tolerated), feeds every datagram
through the **same `Pipeline::ingest` the receive loop uses**, and reports per trcode: count,
length distribution, end-keyword check, what happened (published · filtered · unknown ·
wrong-length · refused), the first refused messages with their cause and head, and per port
which codes arrived. `--dump` writes every published record as one TSV line for comparing
against a reference decoder. No `pcap` crate; a day of 92 GB takes about eight minutes.

This is the check that matters. The decoder tests are built from the standards, and a
standard misread makes the builder and the decoder wrong together — the tests still pass.
The first replay found `B604F` (40 % of a trading day) being refused as wrong-length on every
single message, with all decoder tests green. **Move a layout or a depth, run `jeed-pcap`.**

### `jeed-crypto` — exchange WebSocket feeds

One `[[feed]]` is one WebSocket connection, one thread, one ring. The `venue` picks the
router; the `[[feed.instrument]]` blocks under it are what that connection subscribes to.
Twelve venue names are known:

| `venue`                              | wire venue byte           | address                          |
|--------------------------------------|---------------------------|----------------------------------|
| `binance-spot` / `binance-futures`   | BinanceSpot / BinanceFutures | `wss://stream.binance.com:9443/stream` / `wss://fstream.binance.com/stream` |
| `upbit` / `bithumb`                  | Upbit / Bithumb           | `wss://api.upbit.com/websocket/v1` / `wss://ws-api.bithumb.com/websocket/v1` |
| `okx`                                | Okx                       | `wss://ws.okx.com:8443/ws/v5/public` |
| `bybit-spot` / `bybit-linear`        | BybitSpot / BybitLinear   | `wss://stream.bybit.com/v5/public/spot` / `…/linear` |
| `bitget-spot` / `bitget-linear`      | BitgetSpot / BitgetLinear | `wss://ws.bitget.com/v2/ws/public` |
| `gate-spot`                          | GateSpot                  | `wss://api.gateio.ws/ws/v4/`     |
| `kucoin-spot` / `kucoin-futures`     | KucoinSpot / KucoinFutures | **from a REST ticket** (`bullet-public`); no `url` |

The conf speaks **four channel words** and the router translates them into the venue's
stream names:

| `venue`            | `trade`                                 | `bbo`         | `book`                       | `delta`                                   |
|--------------------|-----------------------------------------|---------------|------------------------------|-------------------------------------------|
| binance-spot / -futures | `@trade` / `@aggTrade`             | `@bookTicker` | `@depth{N}@100ms` (N = 5·10·20) | `@depth@100ms`                          |
| upbit / bithumb    | `trade`                                 | –             | `orderbook`                  | –                                         |
| okx                | `trades`                                | –             | `books5`                     | `books`                                   |
| bybit-spot / -linear | `publicTrade`                         | –             | `orderbook.{N}` (snapshot then deltas) | –                               |
| bitget-spot / -linear | `trade`                              | –             | `books` / `books{N}` (snapshot then deltas) | –                          |
| gate-spot          | `spot.trades`                           | –             | – (start book via REST)      | `spot.order_book_update`                  |
| kucoin-spot / -futures | `/market/match` / `/contractMarket/execution` | –     | – (start book via REST)      | `/market/level2` / `/contractMarket/level2` |

`book` is *whatever the venue's book channel emits* (a full book on Binance and OKX; a
snapshot followed by diffs on Bybit and Bitget). `delta` is a diffs-only channel that needs
a start book: Binance and OKX get it from their `book` channel, Gate and KuCoin fetch it
over REST after every (re)connect and publish it to the same ring as a `Quote`. A channel the
venue does not offer, or a depth it does not serve, is refused by `--check` — the check is
the router's own constructor, so `--check` and the start agree.

```toml
[[feed]]
name  = "binance-spot"
venue = "binance-spot"
mode  = "block"                       # hundreds of messages a second: block is the default
cores = [4]
ring  = "jeed.crypto.binance-spot"
ring_slots = 16384
# url = "wss://…"                     # override the venue default (proxy, testnet)
# ping_secs = 30                      # protocol ping after this much silence; twice it → reconnect
# reconnect_secs = 5

[[feed.instrument]]
symbol = "BTCUSDT"                    # the venue's own spelling, including case
price_decimals = 2                    # tickSize 0.01   — from exchangeInfo, and it changes
qty_decimals   = 5                    # stepSize 0.00001
channels = ["trade", "bbo", "book", "delta"]
# depth = 10                          # book depth where the venue offers a choice

[[feed]]
name  = "kucoin-futures"
venue = "kucoin-futures"
mode  = "block"
cores = [6]
ring  = "jeed.crypto.kucoin-futures"
ring_slots = 16384
# rest = "https://api-futures.kucoin.com"   # REST base override

[[feed.instrument]]
symbol = "XBTUSDTM"
price_decimals = 1
qty_decimals   = 0                    # contracts are whole lots
channels = ["trade", "delta"]
```

`price_decimals` / `qty_decimals` are **configuration**, taken from the venue's instrument
reference. A wrong scale does not round: frames whose digits the scale cannot hold are
refused, and the report line's `fail` counter climbs. Some venue facts the routers already
handle for you: Upbit and Bithumb send JSON in binary frames and switch to exponent notation
above ten million (`1.04525E8`); OKX and Bybit batch several trades per frame; Bybit's
`u == 1` is a full book whatever `type` says; KuCoin timestamps are nanoseconds on trades
and milliseconds on books; deltas with more than 32 changed levels are dropped for the
consumer to resync, never truncated.

Subscriptions, dialect pings (`ping`, `{"op":"ping"}`, `spot.ping`, `{"type":"ping"}`) and
REST requests are all *returned by the router* and performed by the binary between rounds —
the receive loop itself makes no blocking call.

### `jeed-fix` — FIX 4.4 market data

One `[[feed]]` is one FIX session, one thread, one ring. The session — connect, Logon,
heartbeats, TestRequest, ResendRequest, the silence check, reconnect — runs by itself; the
binary sends one MarketDataRequest (`35=V`) after every Logon and otherwise only reports.

```toml
[[feed]]
name  = "smbs"
venue = "smbs"                        # the only FIX venue today
mode  = "block"
cores = [4]
ring  = "jeed.fix.smbs"
ring_slots = 16384
host = "10.0.0.1"                     # from your circuit assignment
port = 9100
sender_comp_id = "JEED"               # tag 49, us
target_comp_id = "SMBS"               # tag 56, the venue
price_decimals = 2                    # FIX carries no precision: the whole session is scaled once
qty_decimals   = 0
symbols = ["USD/KRW", "EUR/KRW"]      # requested and kept; anything else is filtered
# begin_string = "FIX.4.4"   heartbeat_secs = 30   reset_seq = true
# depth = 0                  # tag 264: 0 = full book, 1 = top of book
# subscription = "updates"   # "updates" (snapshot + incrementals) or "snapshot" (once)
# default_qty = 0            # size to publish for a snapshot level that carries none
```

`jeed-fix` the crate knows the protocol and no venue; the binary supplies the venue layer as
a **conf-driven adapter**. A full refresh (`35=W`) becomes a `Quote`, a trade entry in an
incremental (`35=X`) becomes a `Trade`, and two things are **refused rather than guessed**:
an incremental that moves the book (the wire has no delta record for it yet) and a level with
no size when no `default_qty` is configured. Refusals are counted in the report line, so a
venue doing something the adapter does not understand shows up as a number, not as a wrong
book. Venue-specific behaviour is added beside the venue once a live session confirms it; the
session itself has been exercised only against synthetic messages so far.

## Verification

```bash
cargo test --workspace                    # ~1,300 tests. Socket tests use loopback; multicast uses 239.255/16
cargo clippy --workspace --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo test -p jeed-crypto --test recv live -- --ignored   # real exchanges
```

Windows and Linux (WSL) must give the same results. OS calls live only in
`jeed-shm/src/mapping/`, `jeed-krx/src/recv/socket/` and `jeed/src/{cpu,signal}/` as
`windows.rs` / `posix.rs`; nothing above them carries a `#[cfg]`.

Beyond the unit tests, three checks against reality have been done and are repeatable:

- **KRX**: a full trading day (2026-08-07, 192.8 M datagrams) replayed with `jeed-pcap`, and
  316,111 decoded records (10.06 M values) compared against the previous decoder's output —
  zero mismatches (`documents/todo.md` §16).
- **Crypto**: live runs against Binance, Upbit, Gate, KuCoin spot and KuCoin futures; every
  feed connects, subscribes, fetches its start book where needed, and publishes with no
  refusals.
- **FIX**: the adapter and conf against the synthetic SMBS messages in `jeed-fix`'s test
  suite; a live session is still to be run.

`tests/` mirrors `src/`. Message builders and captured frames live once under
`tests/<venue>/common/` and other test crates reach them by `#[path]` rather than copying.

## Repository guide

```text
documents/
  feed_handler.md      design rationale (section numbers referenced from todo.md)
  todo.md              work log · decisions · measurements (pcap scans, loss rates, the replay)
  krx/                 KRX standards excerpts, layout tables, market-rule notes
conf/
  krx_trcodes.toml     generated from the standards (tools/gen_krx_trcodes.py) — do not edit by hand
  krx.example.toml     jeed-krx template; IPs and ports come from the circuit assignment
  crypto.example.toml  jeed-crypto template
  fix.example.toml     jeed-fix template
tools/                 standards (xlsx) → TOML generator, layout dumper
```

## Not yet

- Night-session derivatives (`…V` trcodes): same layouts, different ports, same instrument
  codes with different books — needs a session/board identifier on the wire first.
- Bond decoders against the live circuit; retail bonds, REPO, gold spot; HTX, Kraken.
- A live FIX session; a `SnapshotDelta` mapping for FIX incrementals.
- A blocking fallback for ring consumers (`WaitOnAddress`); a liquidation policy on feed
  death — the consumer's decision.

## License

MIT
