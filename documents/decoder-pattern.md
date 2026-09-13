# Decoder Patterns by Protocol Type

Market data decoders follow fundamentally different structural patterns depending on
the underlying protocol and exchange architecture.

## Pattern Overview

| Pattern | Protocol | Decoder Scope | Instrument Identity | Examples |
|---------|----------|---------------|---------------------|----------|
| Per-Instrument | WebSocket/JSON | 1 decoder = 1 instrument | Implicit (subscription) | Binance, Bybit |
| Per-Message-Type | Binary Multicast | 1 decoder = N instruments | Embedded in payload | KRX/IFX |
| Per-MsgType | FIX (Tag-Value) | 1 decoder = N instruments | Tag field per message | CME, LSE |

## Crypto Exchanges (Per-Instrument)

WebSocket-based crypto exchanges expose per-instrument streams. Each subscription
targets a single symbol, so the decoder naturally becomes instrument-scoped.

**Characteristics:**
- One decoder instance per instrument (e.g., `BinanceSpotQuoteDecoder` holds `instrument_id`)
- Price/quantity extractors stored per-instance (precision varies per instrument)
- No ID resolution needed — the subscription itself defines the instrument
- JSON parsing, variable-length payloads

**Example:** `BinanceSpotQuoteDecoder`
```
struct BinanceSpotQuoteDecoder {
    instrument_id: InstrumentId,    // bound at construction
    max_depth: usize,
    price_extractor: DynamicExtractor,  // per-instrument precision
    qty_extractor: DynamicExtractor,
}
```

Stream URL: `wss://stream.binance.com:9443/ws/fdusdusdt@depth5`
- Stream is already scoped to `fdusdusdt`
- Decoder knows its instrument at construction time

## KRX/IFX (Per-Message-Type, Binary Multicast)

KRX uses the IFX (Information Exchange) protocol — a binary multicast system where
a single feed carries one message type for all instruments.

**Characteristics:**
- One decoder per interface code (e.g., `IFMSRPD0034` for derivative 5-level quotes)
- Instrument identity extracted from ISIN code at fixed byte offset (bytes 17-29)
- `id_alias` maps raw ISIN to `InstrumentId` at parse time
- Extractors from shared `EXTRACTOR_CONTAINER` (uniform layout within a message type)
- Fixed payload length, fixed field offsets, `0xFF` end marker
- TR codes (5 bytes) validate message sub-categories

**Example:** `KrxDerivativeQuoteDecoder` (IFMSRPD0034)
```
struct KrxDerivativeQuoteDecoder {
    payload_length: usize,          // fixed: 324 bytes
    isin_code_range: Range<usize>,  // 17..29
    timestamp_range: Range<usize>,  // 35..47
    quote_start_index: usize,       // 47
}
// No instrument_id field — resolved per message from payload
```

Message layout:
```
Offset  Length  Field
0       2       Data Category
2       3       Information Category
5       8       Message Sequence Number
13      2       Board ID
15      2       Session ID
17      12      ISIN Code  <-- instrument identity
29+     12      Processing Time
41/47+  N       Quote/Trade Data
end-1   1       End Marker (0xFF)
```

**Interface codes in this codebase:**
- `IFMSRPD0023` — Bond quotes (5-level, 462 bytes)
- `IFMSRPD0034` — Derivative quotes (5-level, 324 bytes)
- `IFMSRPD0035` — Stock derivative quotes (10-level, 554 bytes)
- `IFMSRPD0036` — Derivative trades (173 bytes)

## FIX Protocol (Per-MsgType, Tag-Value)

FIX (Financial Information eXchange) is a session-based, tag-value protocol used
globally by traditional exchanges and brokers.

**Characteristics:**
- One decoder per MsgType (`35=W` for snapshots, `35=X` for incremental refresh)
- Instrument identity in tag fields (Tag 55 = Symbol, Tag 48 = SecurityID)
- Tag-based parsing — fields can appear in any order, scanned by tag number
- Variable-length messages with repeating groups
- TCP session with logon, heartbeats, sequence number management

**Example message** (`35=W`, Market Data Snapshot):
```
8=FIX.4.4|35=W|49=EXCHANGE|55=AAPL|268=2|
  269=0|270=150.25|271=1000|   <- bid (269=0)
  269=1|270=150.30|271=500|    <- ask (269=1)
```

Key tags:
- `35` — MsgType (W=snapshot, X=incremental, D=order, 8=execution)
- `55` — Symbol
- `48` — SecurityID
- `268` — NoMDEntries (repeating group count)
- `269` — MDEntryType (0=bid, 1=ask, 2=trade)
- `270` — MDEntryPx (price)
- `271` — MDEntrySize (quantity)

## Comparison: KRX vs FIX

Both are message-type-based (1 decoder handles N instruments), but parsing differs:

| Aspect | KRX/IFX | FIX |
|--------|---------|-----|
| Encoding | Fixed-offset binary | Tag=Value delimited text |
| Field access | Direct byte offset | Tag scanning |
| Payload size | Fixed per interface | Variable per message |
| Transport | UDP multicast (one-way) | TCP session (bidirectional) |
| Instrument ID | ISIN at fixed offset | Tag 55/48, position varies |
| Validation | Payload length + TR code + 0xFF | Checksum (Tag 10) + MsgType |

## Why the Patterns Differ

The decoder pattern is driven by the **multiplexing model**:

1. **Crypto WebSocket** — exchange handles demuxing; each stream = one instrument.
   Decoder is naturally per-instrument since no demuxing is needed.

2. **Binary Multicast (KRX/IFX)** — one feed carries all instruments of a message type.
   Decoder is per-message-format; instrument resolution happens inside `parse()`.

3. **FIX Session** — one TCP connection carries all message types and instruments.
   Decoder is per-MsgType; instrument resolution via tag lookup per message.
