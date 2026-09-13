# Data Engine

The data ingestion pipeline consists of two stages: **DataEngine** (reception and
pre-filtering) and **Broker** (decoding and routing). DataEngine instances receive
raw bytes from network sources -- UDP for KRX, WebSocket for crypto exchanges --
and forward them to the Broker via SPSC ringbufs. The Broker decodes those bytes
into common market data types using exchange-specific decoders and routes the
results to TradingEngine consumers (which own both the order books and the
feature hubs). For the full system context, see fractal-engine `documents/structure/flow.md`.

```
    ┌──────────────────┐     ┌──────────────────┐
    │ DataEngine 0     │     │ DataEngine 1     │
    │ (KRX UDP)        │     │ (Binance WS)     │   ...
    └────────┬─────────┘     └────────┬─────────┘
             │ ReceiverMessage         │ ReceiverMessage
             │ (rtrb SPSC)             │ (rtrb SPSC)
             └────────────┬────────────┘
                          │
                   ┌──────▼──────┐
                   │   Broker    │   Decode + Route
                   └──────┬──────┘
                          │
                 OrderBookMessage
                   (rtrb SPSC)
                          │
                  ┌───────▼────────┐
                  │ TradingEngine  │
                  │ (books + hub)  │
                  └────────────────┘
```

## DataEngine

**Source:** `src/engine/data.rs`

Each `DataEngine` bridges a `MarketDataReceiver` to the Broker. Multiple
instances may exist -- one per exchange feed or network interface -- all
funnelling into the same Broker through independent SPSC ringbufs.

### MarketDataReceiver

**Source:** `src/data/receiver.rs`

`MarketDataReceiver` multiplexes UDP sockets and WebSocket streams into a
single blocking `receive()` call. On Unix, it uses `libc::poll()` with a
pre-allocated `PollFdSet` for efficient blocking multiplexing. This design is
compatible with kernel bypass via `LD_PRELOAD` (e.g. Solarflare OpenOnload).
On non-Unix platforms, it falls back to a non-blocking spin loop with
`thread::yield_now()` between scans.

UDP sources are always checked first (prioritized for lower latency). Each
receive returns a `MarketDataRef` -- either `Udp(UdpReceiveRef)` or
`WebSocket { message, stream_index }` -- referencing the caller-provided
buffer for zero-copy UDP reads.

### Struct Fields

```
DataEngine {
    receiver_index: u8,                           // unique index for ReceiverSource
    receiver: MarketDataReceiver,                  // transport-agnostic receiver enum
    producer: rtrb::Producer<ReceiverMessage>,     // SPSC to Broker
    buffer: Vec<u8>,                               // 64 KiB pre-allocated (RECV_BUFFER_SIZE = 65536)
}
```

### Receive Loop

The `run()` method blocks in a tight loop until a shutdown flag
(`Arc<AtomicBool>`) is set:

1. Call `receiver.receive(&mut buffer)` -- blocks via `poll()` until data
   arrives on any source. For KRX UDP feeds, the receiver applies trcode and
   isincode filters internally before returning; filtered messages are dropped
   and counted inside `KrxUdpReceiver` and never surface here.
2. Stamp `reception_time` via `Timestamp::now()`.
3. Build a `ReceiverMessage` by copying the payload and recording the source.
4. Non-blocking push to the Broker via `producer.push()`. If the ringbuf is
   full, the message is silently dropped to avoid back-pressure stalling the
   network stack.

### Thread Model

Each DataEngine runs on a dedicated thread via `spawn()`. An optional
`core_affinity::CoreId` pins the thread to a specific CPU core for
deterministic latency. The thread name follows the pattern
`data-engine-{receiver_index}`.

### ReceiverMessage

The envelope transported from DataEngine to Broker:

```
ReceiverMessage {
    payload: Vec<u8>,           // copied payload bytes
    source: ReceiverSource,     // which receiver/socket produced this
    reception_time: Timestamp,  // nanosecond-precision system time at reception
}
```

`ReceiverSource` identifies the origin:

- `Udp { receiver_index: u8, socket_index: u8, source_addr: SocketAddr }` --
  KRX UDP datagram with network source address.
- `WebSocket { receiver_index: u8, stream_index: u8 }` -- crypto exchange
  WebSocket frame.

## KRX Pre-Filtering

**Source:** `src/data/receiver/krx_udp.rs` (`KrxUdpReceiver`)

KRX UDP feeds carry messages for all instruments on the exchange. Most messages
are irrelevant to the trading system. Pre-filtering inside `KrxUdpReceiver` --
the earliest possible point in the pipeline, before bytes ever leave the
receiver -- drops these messages before they consume `DataEngine`, Broker, or
decoding CPU. `DataEngine` has no awareness that filtering occurs; it simply
receives a stream of pre-filtered messages.

### Filter Configuration

Filters are configured via builder methods on `KrxUdpReceiver` BEFORE wrapping
the receiver in `MarketDataReceiver::KrxUdp`:

```
let receiver = KrxUdpReceiver::new()
    .with_trcode_filter(allowed_trcodes)
    .with_isincode_filter(allowed_isincodes);
let market_receiver = MarketDataReceiver::KrxUdp(receiver);
```

Both filter methods take an `AHashSet` of fixed-size byte arrays. Either or
both can be configured; both default to `None` (no filtering).

### trcode Filter

The 5-byte transaction code (`trcode`) occupies bytes `0..5` of every KRX UDP
message (`KRX_TRCODE_RANGE`, internal to `krx_udp.rs`). When a `trcode_filter`
is configured, `KrxUdpReceiver::receive()` extracts bytes `0..5` of each
incoming UDP datagram into a `[u8; 5]` array and checks it against an
`AHashSet<[u8; 5]>` allow-set. If the trcode is not in the set, the message
is dropped from the receive scan, `trcode_drops` is incremented, and the
receive loop continues polling.

Common KRX trcodes:

| trcode      | Message Type                 |
|-------------|------------------------------|
| IFMSRPD0034 | Derivative quote             |
| IFMSRPD0035 | Stock derivative quote       |
| IFMSRPD0036 | Derivative trade             |
| IFMSRPD0037 | Derivative trade+quote       |
| IFMSSTD0005 | Investor statistics          |
| IFMSRID0007 | Investor statistics (alt)    |
| IFMSRID0008 | Open interest                |
| IFMSRPD0023 | Bond (KTS) quote             |
| IFMSRPD0027 | Bond (KTS) trade             |
| IFMSRPD0029 | Bond (KTS) trade+quote       |

### isincode Filter

The 12-byte ISIN code (`isincode`) occupies bytes `17..29`
(`KRX_ISINCODE_RANGE`, internal to `krx_udp.rs`) in the majority of KRX
message types (quote, trade, trade_quote). When an `isincode_filter` is
configured, `KrxUdpReceiver::receive()` extracts bytes `17..29` into a
`[u8; 12]` array and checks it against an `AHashSet<[u8; 12]>` allow-set.
If the isincode is not in the set, the message is dropped and
`isincode_drops` is incremented.

Messages shorter than 29 bytes bypass the isincode filter. Open interest
messages use a different ISIN offset (`5..17`) but are low-frequency and
pass through the standard filter.

### Filter Bypass

WebSocket messages returned from `KrxUdpReceiver::receive()` bypass all KRX
filters unconditionally. Crypto exchanges (handled by `CryptoReceiver` in
Phase 186) use their own subscription-based filtering at the WebSocket
level, so no DataEngine-side filtering is needed.

### Drop Counter Diagnostics

`KrxUdpReceiver::trcode_drops()` and `KrxUdpReceiver::isincode_drops()`
return the running count of UDP messages dropped by each filter. These are
queryable from outside the receive thread but require reaching into the
enum variant:

```
if let MarketDataReceiver::KrxUdp(rx) = data_engine.receiver() {
    let dropped = rx.trcode_drops() + rx.isincode_drops();
}
```

There is no automatic logging on drops -- counters are read on demand only.
A pure-helper variant `accepts_udp_payload(&self, payload: &[u8]) -> bool`
is also exposed for tests and external diagnostics; it does not increment
counters.

## Broker

**Source:** `src/engine/broker.rs`

The Broker is a single-threaded hub that consumes `ReceiverMessage` values
from all DataEngine channels, decodes them into common types, and routes the
results to downstream engines.

### Struct Fields

```
Broker {
    consumers: Vec<rtrb::Consumer<ReceiverMessage>>,              // one per DataEngine
    receiver_decoder_map: Vec<usize>,                             // receiver_index → decoder_sets index
    decoder_sets: Vec<DecoderSet>,                                // pre-instantiated decoders
    buffers: DecoderBuffers,                                      // reusable decode output buffers
    engine_producers: Vec<rtrb::Producer<OrderBookMessage>>,            // one per TradingEngine
    trading_engine_route_map: AHashMap<InstrumentId, Vec<usize>>,       // instrument → TE indices (1:N)
    shutdown: Arc<AtomicBool>,
}
```

### Spin Loop

The `run()` method is a non-blocking spin loop (no sleep, no yield). It
polls all receiver consumers in round-robin order, draining up to
`BATCH_SIZE = 64` messages per consumer per round before moving to the next.
The loop exits when the shutdown flag is set and a full round produces zero
messages (all receivers have been drained).

The Broker is spawned on a dedicated thread with mandatory core affinity
(`spawn(core_id)`) to guarantee it never contends with DataEngine or trading
threads for the same CPU core.

### Message Processing

For each `ReceiverMessage`, the Broker:

1. Looks up the `DecoderSet` via `receiver_decoder_map[consumer_idx]`. This
   maps each receiver index to the correct exchange-specific decoder set,
   established at startup from `BrokerConfig`.
2. Dispatches to the appropriate decode method based on `ReceiverSource`:
   - `Udp` sources go to `decode_krx_derivative()` or `decode_krx_bond()`
     depending on the `DecoderSet` variant.
   - `WebSocket` sources go to `decode_binance()` (indexed by
     `stream_index`).
3. If decoding succeeds and produces a `DecodedMessage`, routes it downstream.

## Decoding

**Source:** `src/data/decoders.rs`

### DecoderSet

`DecoderSet` is an enum with one variant per exchange or market segment. Each
variant holds all pre-instantiated decoders for that receiver channel:

| Variant          | Decoders                                                | Extras                        |
|------------------|---------------------------------------------------------|-------------------------------|
| `KrxDerivative`  | quote, stock_quote, trade, trade_quote, investor_stats, open_interest | `id_alias`, `quote_level_map` |
| `KrxBond`        | quote, trade, trade_quote                               | `id_alias`, `quote_level_map` |
| `BinanceSpot`    | `Vec<BinanceStreamDecoder>` (indexed by stream)         | `quote_level_map`             |
| `BinanceFutures` | `Vec<BinanceStreamDecoder>` (indexed by stream)         | `quote_level_map`             |
| `Bybit`          | orderbook, trade, order_decoder, execution_decoder      | --                            |
| `Upbit`          | orderbook, trade, ticker, order_decoder                 | --                            |
| `Bithumb`        | orderbook, trade, ticker, order_decoder                 | --                            |

### KRX Trcode Dispatch

KRX messages are dispatched by extracting the 5-byte trcode from bytes
`0..5` and testing each decoder's `is_valid_trcode()` in frequency order
(quote first, then trade, trade_quote, stock_quote, investor_stats, open
interest). The first match decodes the payload into the corresponding
`DecoderBuffers` field. If no decoder matches, `DataError::InvalidTrCode` is
returned.

### Binance Stream Dispatch

Binance messages are dispatched by `stream_index` -- each WebSocket stream
corresponds to one symbol and one data type. `BinanceStreamDecoder` is an
enum with eight variants covering spot and futures data:

- `BestQuote` / `FuturesBestQuote` -- `@bookTicker` streams
- `Quote` / `FuturesQuote` -- partial book depth (`@depth5`/`@depth10`/`@depth20`)
- `Delta` / `FuturesDelta` -- order book diff (`@depth`)
- `Trade` / `FuturesTrade` -- aggregate trades

### ISIN Resolution

KRX messages identify instruments by ISIN code, not by the system's internal
`InstrumentId`. Each KRX `DecoderSet` variant carries an `IdAliasMap`
(`id_alias`) that maps 12-byte ISINs to `InstrumentId` values. Decoders
resolve the ISIN during decoding, so downstream consumers receive messages
tagged with the correct `InstrumentId`.

A `QuoteLevelMap` maps each `InstrumentId` to its configured order book depth
level, used by quote decoders to parse the correct number of price levels.

### DecoderBuffers

```
DecoderBuffers {
    quote: QuoteData,
    trade: TradeData,
    trade_quote: TradeQuoteData,
    open_interest: OpenInterestData,
    investor_stats: InvestorStatisticsData,
    quote_delta: QuoteDeltaData,
    ticker: UpbitTickerData,
}
```

A single `DecoderBuffers` instance is reused across all messages. Decoders
populate fields by `&mut` reference -- no heap allocation per decode. After
routing, the buffer contents are cloned into output channels (see below).

### DecodedMessage

`DecodedMessage` is a lightweight tag enum indicating which `DecoderBuffers`
field was populated:

`Quote` | `Trade` | `TradeQuote` | `OpenInterest` | `InvestorStatistics` |
`QuoteDelta` | `Ticker`

This avoids copying decoded data until routing confirms a downstream consumer
exists.

## Routing

**Source:** `src/engine/broker.rs` (`route_message()` and helper methods)

After decoding, the Broker routes each `DecodedMessage` to downstream
engines using the route map:

- `trading_engine_route_map: AHashMap<InstrumentId, Vec<usize>>` -- maps
  each instrument to one or more TradingEngine indices (1:N). Must cover
  every instrument the TE-owned FeatureHubs consume, including hub-only
  feature inputs the strategies never trade.

### Output Message Type

**`OrderBookMessage`** (sent to TradingEngine):
`Quote(QuoteData)` | `Trade(TradeData)` | `TradeQuote(TradeQuoteData)` |
`OpenInterest(OpenInterestData)` | `SnapshotDelta(SnapshotDeltaData)` |
`InvestorStatistics(InvestorStatisticsData)` | `TriggerRecovery(InstrumentId)`

`InvestorStatistics` feeds the TE-owned FeatureHub only (never the order
books) and is broadcast to all TradingEngines, each hub gating by
`valid_invstatid`.

### Routing Rules

| DecodedMessage         | TradingEngine                  |
|------------------------|--------------------------------|
| `Quote`                | By instrument via route map    |
| `Trade`                | By instrument via route map    |
| `TradeQuote`           | By instrument via route map    |
| `OpenInterest`         | By instrument via route map    |
| `InvestorStatistics`   | Broadcast to ALL TradingEngines|
| `SnapshotDelta`        | By instrument via route map    |
| `ChecksumMismatch`     | `TriggerRecovery` via route map|

**`InvestorStatistics`** is broadcast to every FeatureEngine channel (not
routed by instrument) because it contains market-wide data aggregated across
instruments.

**`Ticker`** (24h market summary from Upbit/Bithumb) is not routed to either
engine -- it is available for gateway-level consumption only.

### PriceUpdate to ExecutionEngine

The architecture defines a `PriceUpdate { instrument, price }` channel from
the Broker to ExecutionEngine via a dedicated rtrb SPSC ringbuf for
mark-to-market PnL calculation. The channel is allocated by the Orchestrator
at startup.

### Clone Optimization

When routing to multiple engines for the same instrument, the last channel
in each route receives the original message (moved), while prior channels
receive clones. This saves one `clone()` per multi-engine route.
