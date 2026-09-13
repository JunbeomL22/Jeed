# Feed Handler 분리 설계

OMS(주문 + TE BOOK)와 Feed Handler 를 **별도 프로세스**로 쪼개고, 정규화된 시세를
**공유 메모리**로 넘기는 구조. 2026-09-12 설계.

## 1. 구성

```
krx-udp handler (proc) ──shm ring──┐
                                   ├──▶ OMS proc ──▶ strategy thread A
fix handler     (proc) ──shm ring──┘                ──▶ strategy thread B
                                        (TE BOOK 소유)
parquet replay (backtest) ─────────▶ 같은 와이어 레코드
```

- Feed handler 는 **무상태**다. 디코드 + 필터 + 정규화만 하고 북을 안 갖는다.
  재시작이 싸고, 대신 OMS 쪽에 북 복구 경로가 필요하다.
- OMS 가 북과 주문을 함께 갖는다. 북의 단일 writer.
- **receiver(라우터) 프로세스·스레드는 두지 않는다.** 근거는 §2.

## 2. 라우터를 두지 않는 이유

- 필터는 링 **앞**이 싸다. `KrxUdpReceiver` 의 `trcode_filter`/`isincode_filter` 가
  이미 `receive()` 안에서 떨구고 있어 그대로 맞다.
- 시간 정렬은 라우터로 못 푼다 — 느린 피드를 기다려야 하고 그건 곧 지연이다.
  라이브는 도착 순서대로 처리한다(백테스트의 병합은 `reader` 계층 일이지 라이브 배선이 아니다).
- 라우터는 직렬화 지점이자 피드 간 head-of-line 블로킹 지점이 된다.
- 코어 예산: 16물리/32논리(Ryzen 9 9950X, CCD 2장)에서 핀할 스레드가 feed 2 + 전략 2 =
  **4물리**. 코어 수만 보면 라우터 1물리는 이제 감당된다 — **이 논거는 9950X 에서 약해졌고,
  남는 세 줄이 라우터를 거부하는 진짜 이유다.** 대신 새 제약이 생겼다: 링 라인이 CCD 를
  건너면 슬롯마다 Infinity Fabric 왕복이라 **생산자와 그 소비자는 같은 CCD 에 핀한다.**

라우터가 값을 하는 조건은 **전략 스레드가 8개 이상**이거나 **북 재구성을 공유**해야 할 때뿐이다.

전략 스레드 2개는 같은 shm 링을 **각자 커서로 직접** 읽는다. 한 스레드가 읽어서
rtrb 로 다른 스레드에 넘기는 건 라우터 부활이므로 금지.

## 3. IPC

- **shared memory 링.** named pipe 는 메시지당 syscall(µs)이라 시세에 쓰지 않는다. 제어 채널용.
- 프로세스 경계 자체는 지연 비용이 아니다. mmap 된 메모리는 그냥 메모리라
  writer→reader 캐시라인 왕복 비용은 rtrb 와 같다.
- **피드당 링 하나, 리더는 N개다** (SPMC 브로드캐스트). 처음엔 "피드당 SPSC 링 2개" 로
  적었는데, 그게 피하려던 비용(덮어쓰기·랩어라운드 처리)을 결국 §7 때문에 다 치르게 됐다 —
  백프레셔를 안 쓰기로 한 순간 SPSC 여도 덮어쓰기 링이고 슬롯 seqlock 이 필요하다. 그러고 나면
  리더를 하나로 제한해서 얻는 게 없다. 지금 구조:
  - 공유 헤더에 **읽기 커서가 없다** — `write_cursor` 와 `drop_counter` 뿐. 커서는
    `RingConsumer.next_seq`, 프로세스 로컬 필드다.
  - 소비자는 **read-only 로 매핑**하고 등록도 배타성도 없다. 붙는 것만으로 끝이고, 생산자는
    리더가 몇인지 모른다. 읽기는 공유 상태에 아무것도 쓰지 않으므로 리더가 늘어도 생산자
    코어에 캐시라인 경합이 안 생긴다.
  - 그래서 **소비자별 백프레셔 분리가 자동**이다. 소비자 A 가 lag 16000 이고 B 가 3 이어도
    서로 모른다. 이게 §2 가 라우터를 거부한 것과 같은 성질이다 — 느린 쪽이 빠른 쪽을 막는
    지점이 하나도 없다.
  - 생산자 쪽만 여전히 하나다. `RingProducer` 는 `Send` 지만 `!Sync` 고, 두 번째 생산자는
    `reused_existing_section()` 이 잡는다.
  (참고: `src/utilities/broadcast.rs` 의 `Broadcast<T>` 는 *최신값* 브로드캐스트라
  중간값 유실이 정상인 자료구조다. 시세 스트림용이 아니다.)
- 진짜 락인은 링이 아니라 **레코드 포맷**이다. `#[repr(C)]` 고정 크기로 잡아두면
  나중에 링만 갈아끼운다.
- Windows: `CreateFileMapping(INVALID_HANDLE_VALUE, …)` + `MapViewOfFile` 로
  **페이지파일 백업 named mapping**. 파일 백업 mmap 은 플러시가 끼어 피한다.
  블로킹 폴백이 필요하면 `WaitOnAddress`/`WakeByAddressSingle`.
- Linux: `shm_open` + `ftruncate` + `mmap(MAP_SHARED)` — `/dev/shm` 은 tmpfs 라
  마찬가지로 파일 백업이 아니다. **수명 규칙만 반대다:** 윈도우 섹션은 마지막 핸들과 함께
  죽고, POSIX 이름은 `shm_unlink` 전까지 남는다. 그래서 리눅스에선 "이름이 이미 있다"가
  살아 있는 생산자를 뜻하지 않는다 — 재기동 판별은 §7 대로 `boot_id` 가 한다.

### 3.1 링 크기 — L3 를 벗어나지 않는 것이 상한이다

슬롯 하나는 `WIRE_RECORD_LEN` = **640 B** 고, 링은 그 배열 그 자체다(직렬화 없음).
`Slot` 이 `DerefMut<Target = WireRecord>` 라 디코더가 공유메모리에 직접 쓴다 — 스테이징
복사가 없고, 링이 디코드 위에 얹는 명령어는 x86 에서 평범한 `mov` 몇 개뿐이다(원자적 RMW 0회).
**쓰기의 실제 비용은 캐시라인 10 개를 건드리는 것뿐이고, 그래서 크기 결정은 캐시 문제다.**

| `ring_slots` | 크기 | |
|---:|---:|---|
| 4,096 | 2.5 MiB | cold (초당 수 건) |
| **16,384** | **10 MiB** | 기본값 |
| 65,536 | 40 MiB | **쓰지 않는다** |

상한은 **한 CCD 의 L3 = 32 MiB** 다(§2). 9950X 의 L3 는 합이 64 MiB 지만 CCD 당 32 MiB 로
갈린 것이지 통합 캐시가 아니다. 40 MiB 링은 그 32 를 넘어서:

- 생산자가 쓰는 슬롯은 한 바퀴 전에 evict 된 라인이라 **매 레코드가 DRAM RFO + writeback** 이고,
- 그 스트림이 같은 L3 를 쓰는 **소비자의 북을 계속 밀어낸다.**

대역폭이 모자라서가 아니다 — 200k rec/s 라도 256 MB/s 로 DDR5 앞에서 반올림 오차다. 문제는
**재사용 거리**다. 링이 L3 안에 있으면 생산자의 쓰기도 소비자의 읽기도 L3 히트고, 넘기는
순간 링 자체가 캐시 오염원이 된다.

### 3.2 링을 키워서 사는 것은 지연이 아니라 **정지 허용 시간**이다

세 숫자를 섞지 않는다:

| | 값 | 무엇 |
|---|---:|---|
| 종단 지연 | **< 1 µs** | 디코드 + 슬롯 쓰기 + `write_cursor` 한 번 튕김 + 소비자 폴 |
| 랩 예산 | `capacity ÷ 레이트` | 소비자가 **죽어 있어도** 되는 시간 |
| 따라잡기 | `capacity ×` ~75 ns | 밀린 걸 실제로 읽는 시간 |

`ring_slots = 16384` 이면 랩 예산이 실측 평균 3,740 rec/s(§11)에서 **4.4 초**, 따라잡기는
**1.2 ms** 다. 예산이 따라잡기의 3,500 배라는 게 이 설계가 사는 이유다 — 소비자가 잠깐
밀리는 건 경주가 아니다.

그리고 랩 예산은 **정상 지터가 아니라 사고 대비**다. 핀된 스핀 소비자의 스케줄 지터는 µs,
페이지 폴트는 수십 µs 다. 4.4 초를 태우려면 핫 루프 안의 블로킹 호출(디스크 로깅, 주문 왕복),
핀 누락, 디버거 정지, 프로세스 재기동 같은 것이 있어야 한다. 그러니 "지연을 줄이려고" 링을
키우는 건 착오고, 사는 것은 **"소비자가 사고를 쳐도 리싱크 없이 살아남을 창"** 이다.

> 아직 안 잰 것: **개장 버스트의 초당 최대 발행 건수.** §11 의 3,740 rec/s 는 야간장을 포함한
> 11 시간 평균이고, `B604F` 하나가 하루의 40% 라 09:00 피크는 그보다 훨씬 위일 것이다.
> `jeed-pcap` 리포트에 초 경계 카운터 한 줄이면 나온다. 그 숫자가 나오기 전까지 16384 는
> "L3 안에서 최대한" 이지 "측정으로 고른 값" 이 아니다.

### 3.3 밀린 소비자는 **최신부터** 읽는다

랩을 넘겨 유실이 확정되면(`Recv::Lagged`) 커서는 **라이브 엣지**로 간다. 살아남은 가장
오래된 레코드로 되감지 않는다.

되감으면 소비자가 낡은 한 바퀴를 통째로 재생하면서 라이브로 기어온다. 읽기만 보면 1.2 ms 라
싸 보이지만 레코드마다 북을 갱신하면 수십 ms 동안 **몇 초 묵은 시세를 처리하느라 라이브에
눈이 먼다** — 이미 잃은 것을 따라잡느라 지금 가격을 못 보는 것이다. `RingConsumer::attach`
가 라이브 엣지에서 시작하는 이유가 그대로 여기에 걸린다.

잃은 것은 **`Lagged(n)` 의 `n`** 으로 세어서 알린다. 반응은 §7 대로 kind 가 가른다 —
자가치유 kind 는 카운터만(다음 스냅샷이 고친다), `SnapshotDelta` 는 리싱크. **되감아 재생해도
델타 사슬의 구멍은 안 메워지므로**, 재생이 사주는 것은 체결 테이프의 완전성뿐이고 그건
라이브를 늦추면서까지 살 것이 아니다.

`Lagged(0)` 은 다르다 — 생산자가 덮어쓰는 슬롯을 읽다 물러난 것이라 **아직 한 건도 안 잃었다.**
유실의 조기 경보이므로 카운터만 올리고 리싱크는 걸지 않는다.

## 4. 세 층 — 와이어 타입은 정규화 포맷과 별개다

```
[원시 바이트]   KRX UDP 바이너리 / FIX tagvalue / parquet 행
      ↓ decode                        (feed handler, backtest reader)
[와이어 레코드]  #[repr(C)] · 고정 크기 · 버전 있음 · shm ABI
      ↓ adapt                         (OMS 진입점, 구현 하나)
[정규화 포맷]    SnapshotData / TradeData · 프로세스 안 Rust 타입
      ↓
[북 / 피처 / 전략]
```

와이어 레코드는 **두 바이너리 사이의 ABI** 다. 독립 배포·재시작되므로 버전이 붙고
레이아웃이 고정돼야 하며, 안 맞으면 조용히 잘못 읽는 게 아니라 **터져야** 한다.
도메인 모델(`SnapshotData`)을 거기 묶으면 의미상 리팩터마다 호환성이 깨진다.

**`SnapshotData` 는 건드리지 않는다.** serde / `DeepSizeOf` / parquet / EDA 에 엉켜 있고
`Vec` → 고정 배열은 백테스트 메모리 프로파일을 바꾼다.

### live/backtest 괴리 차단

지금 정규화 경로가 둘이고 **사람이 손으로 맞추고 있다**:

```
live:     KRX 디코더 ──────────────────────────────▶ SnapshotData
backtest: parquet → KrxQuoteRow(f64) → fill_quote_buffer ─▶ SnapshotData
```

→ **2026-09-13 전환 완료.** 지금은

```
live:     KRX 디코더 ─▶ WireRecord ─┐
backtest: parquet 리더 ─▶ WireRecord ─┴─ wire::adapt::fill_snapshot / fill_trade ─▶ SnapshotData / TradeData
```

`KrxQuoteRow`·`KrxTradeRow`·`KrxDayEvent` 는 삭제됐다. 리더(`read_krx_day`)가 f64→정수 변환
(`KrxWireScales`)을 하고 raw ISIN 을 실은 레코드를 내며, 피드(`KrxParquetFeed::next_event`)는
`producer_seq` 를 찍고 `quote_levels` 로 `depth` 를 덮어쓴 뒤 어댑터에 넘긴다. SMBS 는 `Venue::Smbs`
레코드 + 티커를 12바이트로 채운 키를 1항목 alias map 으로 푼다. 피드 캐시는 `fe.feed/3`(레코드
바이트 이미지 그대로, 스케일이 지문에 포함).

`src/backtest/feed.rs:514` 의 주석이 "*mirroring the live KRX quote decoder's buffer
contract*" 라고 적고 있다. **backtest 리플레이도 와이어 레코드를 생산**하게 해서
두 경로가 어댑터 입력에서 같아지게 한다. 어댑터는 `fill_quote_buffer` 자리 하나.

> ⚠️ 전환 시 f64→정수 반올림 지점이 옮겨진다. 기준점 conf 하나로 **A/B 해서 성적이
> 동일한지 확인**하고 넘어갈 것. 안 맞으면 반올림 규칙부터 맞춘다.

## 5. 와이어 레코드

### segment 헤더 (링 앞 1회)

`magic` · `format_version` · `record_size` · `boot_id` · `write_cursor` · `drop_counter`

- `format_version` 불일치는 attach 에서 **거부**한다.
- `boot_id` 는 feed 재시작 감지용. 바뀌면 리더가 `producer_seq` 기대값을 리셋하고
  동시에 **북이 stale** 임을 안다.

### 레코드 헤더 48B

| 필드 | 크기 | 비고 |
|---|---|---|
| `kind` | u8 | Quote / Trade / TradeQuote / OpenInterest / InvestorStats / SnapshotDelta / PriceLimit / MarketSchedule / **Heartbeat** |
| `venue` | u8 | `Venue` 는 필드 없는 enum(Krx=0 / Nxt=1 / Smbs=2) → u8 |
| `flags` | u8 | §8 |
| `depth` | u8 | 유효 레벨 수 (≤10 — 주식·ETF 채널 01S 가 10단) |
| `price_scale` / `qty_scale` | u8 ×2 | `Scale` 은 명시 discriminant(`S0=0`)라 u8 안전 |
| `_pad` | u16 | |
| `producer_seq` | u64 | feed→OMS **링** 유실 감지 (§7) |
| `recv_ns` | u64 | feed 수신시각. OMS 의 `system_time` 이자 alias resolve 입력 |
| `venue_ns` | u64 | **조립된 절대 ns** (§6) |
| `isin` | [u8;12] | `Isin = [u8; ISIN_LEN]`, `ISIN_LEN = 12` |
| `_pad1` | [u8;4] | 명시 패딩(0). 오프셋: kind 0 · venue 1 · flags 2 · depth 3 · scales 4-5 · producer_seq 8 · recv_ns 16 · venue_ns 24 · isin 32 |

> **Jeed 에서는 헤더가 64B 다** (`WIRE_FORMAT_VERSION = 2`, 2026-09-13). `isin: [u8;12]` 이
> `symbol: [u8;24]` 이 되고 `_pad1` 이 8바이트가 됐다 — 크립토 심볼이 12바이트에 안 들어간다
> (`1000000MOGUSDT` 14, OKX 옵션 `BTC-USD-240329-70000-C` 22). **레코드는 640B 그대로**:
> 헤더가 꼬리 패딩(48→32)을 먹었을 뿐이다. `Venue` 에는 `BinanceSpot = 3` ·
> `BinanceFutures = 4` 가 붙었다 — 같은 `BTCUSDT` 가 두 시장에서 다른 종목이라 베뉴를
> 공유하면 식별자가 충돌한다.
>
> **v3 (2026-09-13)** 은 레이아웃을 안 건드리고 베뉴만 늘렸다: `Upbit = 5` ·
> `Bithumb = 6` · `Okx = 7` · `BybitSpot = 8` · `BybitLinear = 9`. **베뉴를 몇 개 쓰느냐는
> 모듈을 몇 개 쓰느냐와 다른 질문이다** — OKX 는 `instId` 가 이미 시장을 가르므로
> (`BTC-USDT` / `BTC-USDT-SWAP` / `BTC-USD-240329-70000-C`) 전 시장이 바이트 하나고,
> 바이비트는 전문이 스팟·리니어 공통인데도 `BTCUSDT` 가 충돌하므로 둘이다. 소비자가 모르는
> 베뉴 바이트는 이미 거부되는 레코드이므로(`Venue::from_u8`), 버전을 올리는 건 레이아웃
> 통보가 아니라 "이제 거부하지 말라" 는 신호다.
>
> **v4 (2026-09-13)** 도 레이아웃을 안 건드리고 베뉴만 늘렸다: `BitgetSpot = 10` ·
> `BitgetLinear = 11` · `GateSpot = 12` · `KucoinSpot = 13` · `KucoinFutures = 14`.
> 쿠코인이 앞의 규칙에서 유일하게 애매한 자리다 — 선물이 자산 이름을 바꾸고 접미사를 붙여서
> (`XBTUSDTM` vs 현물 `BTC-USDT`) **지금은** 충돌하지 않는데, 그건 거래소가 문서로 약속한
> 규약이 아니라 작명 습관이다. 틀렸을 때의 대가가 두 오더북을 하나로 합치는 것이라 둘로 갈랐다.
> HTX 와 크라켄은 이 판에서 빠졌다(`todo.md` §6) — 바이트도 안 잡아 뒀다. 안 쓰는 베뉴
> 바이트를 미리 박아 두면 그게 곧 ABI 이고, 나중에 번호를 고치는 건 공짜가 아니다.

### 페이로드 528B · 레코드 576B (구현 2026-09-13, `src/data/common/wire/`)

> **Jeed 에서는 페이로드 544B · 레코드 640B 다.** `TradePayload` 가 32 → 48B 로 늘었다
> (`dyn_upper`/`dyn_lower` + `DYN_LIMIT_VALID`) — `G7` 의 동적상하한가를 버릴 수 없어서다.
> 근거는 `documents/todo.md` §12. 아래 숫자는 이 문서가 쓰인 시점의 것이다.

설계 시점의 "Quote 240B / 합 288B" 는 **TradeQuote(체결+호가) 와 레벨 확장이 안 들어가서**, 그리고
**깊이 5 가 주식·ETF 채널(01S, KODEX 등)의 10단을 못 담아서** 바뀌었다. 파생·채권은 10 중 5 만 쓴다:

- **레벨 24B** — `price: i64` · `qty: u64` · `order_count: u32` · `ext: u32`. `order_count` 유효 여부와
  `ext` 의 의미는 레벨마다가 아니라 **페이로드 단위**로 준다(채널 성격이라 레벨별로 다를 이유가 없다):
  `level_flags` (ORDER_COUNT_VALID), `level_ext_kind` (NONE / BOND_YIELD=i32 / LP_QUANTITY=u32).
  `LevelExtension` 의 유니언이 이것이다.
- **Quote 496B** = 2측 × 10단 × 24B + `quote_ext: u64` + `level_ext_kind` · `quote_ext_kind`
  (NONE / LP_HOLDINGS / SEQUENCE) · `level_flags` + pad 5.
- **Trade 32B** = `price` · `qty` · `cumulative_qty` · `trade_yield: i32` · `trade_kind`
  (NONE=0 / SELL / BUY / UNKNOWN — "채널에 구분 없음" 이 별도 값) · `trade_flags` (CUM_VALID / YIELD_VALID) + pad 2.
- **TradeQuote 528B** = Trade + Quote. 이게 최대라 **페이로드 유니언 = 528B**.
- OpenInterest 8B · InvestorStats 48B(원시 상품/투자자 코드 바이트, OMS 가 `InvStatId` 로 매핑) ·
  PriceLimit 40B · MarketSchedule 72B(범위 필드 원문 보존) · Heartbeat 16B(`received`/`forwarded`
  카운터 — "조용한 시장" 과 "필터가 다 걸러냄" 구분).
- **레코드 = 48 + 528 = 576B, `align(64)`** — 9 캐시라인, tail 패딩 0 (합이 안 맞으면 명시 tail 이 생긴다). 링이 배열이 되고 커서가 인덱스가 된다.
- kind 0 은 미사용 → **0 으로 채워진 슬롯은 무효**. ~~`SnapshotDelta` 는 kind 만 예약,
  페이로드 미정의.~~ → **Jeed v2 에서 정의됐다** (2026-09-13, binance `@depth` 를 받으면서):
  `SnapshotDeltaPayload` 544B = 16B 델타 레벨 × 32단(양쪽 공유) + `U`/`u`/`pu` + 카운트.
  32단을 넘는 메시지는 **자르지 않고 프레임을 버린다** — 자른 델타는 소비자가 완전한 것으로
  받아들이고 영영 못 고친다. 버리면 갱신ID 사슬에 구멍이 나고 소비자가 리싱크한다.
  `pu` 슬롯(`prev_final_update_id` + `PREV_FINAL_VALID`)은 OKX `prevSeqId` 도 쓴다 —
  *이 메시지가 어느 것을 따라야 하는가* 라는 같은 질문의 답이다. 바이비트와 바이낸스 스팟은
  그런 필드를 안 보내므로 슬롯이 **비어 있고**, 0 이 아니다: "이 베뉴는 선행 메시지를 명시하지
  않는다" 와 "선행 메시지가 0번이다" 는 다른 말이다.
- 시각은 프로젝트 타입 그대로 `u64`(`UnixNano`); 차는 `saturating_sub`(`RecordHeader::venue_age_ns`).
- 세그먼트 헤더 128B(2 캐시라인): 첫 줄 상수(magic `FE_WIRE1` · version · record_size · capacity · boot_id),
  둘째 줄 producer 커서(`write_cursor` Release/Acquire · `drop_counter`). `check()` 가 version/record_size
  불일치를 거부한다.

모든 구조체는 **암묵 패딩 0**(필드 폭 합 == size 를 컴파일 타임 단언) — 그래서 `as_bytes()` 가 건전하고
유니언 어느 멤버를 읽어도 유효하다. 어댑터(`wire::adapt::fill_*`)는 §4 의 "구현 하나" 자리다: 헤더
`(venue, isin)` 을 `AliasMap::resolve(isin, recv_ns)` 로 풀고, 라이브 디코더의 버퍼 계약(depth 만큼 채우고
그 밖은 default 로 리셋)을 그대로 따른다. venue 가 alias map 의 venue 와 다르면 오류.

### 기존 타입에서 바뀌는 것

(2026-09-13 전환 완료 — 아래 표의 "지금" 열은 삭제된 `KrxQuoteRow` 를 말한다. `[Option<OrderCount>; N]` 은
페이로드 단위 `ORDER_COUNT_VALID` 플래그로 갔다: 실 DB 는 count 열이 있는 채널에서 NULL 셀이 0건이라 무손실.)

| 지금 | 와이어 |
|---|---|
| `instrument: InstrumentId` | `venue: u8` + `isin: [u8;12]` (§6) |
| `venue_time: Option<UnixNano>` | 값 + `VENUE_TIME_VALID` 플래그 |
| `[Option<OrderCount>; N]` | 값 배열 + 센티널/플래그 |
| `[f64; N]` 가격·수량 | **`BookPrice`(i64) + `Scale`** — 변환이 parquet 리더로 밀리고 live 경로엔 변환이 없어진다 |

`LevelExtension` / `QuoteExtension` 은 데이터 캐리 열거형이라 명시 태그 + 유니언으로 재정의.
참고로 `src/` 전체에 현재 `repr(C)` 는 **0건**이다.

## 6. 식별자와 시각

### `InstrumentId` 는 프로세스 경계를 못 넘는다

`declare_interned_id!`(`src/types/identifier/interned_id_macro.rs:32`)의 `{ id: u64 }` 가
**프로세스 로컬 `AtomicU64`** 에서 나온다 — 인터닝 순서가 곧 id 라, 프로세스마다 같은 ISIN 이
다른 id 를 받는다. `to_arc_str()` 는 `Mutex` 라 핫패스 불가.

→ **와이어에는 `(venue, isin)` 원본을 싣고 OMS 가 해석한다.**

`AliasMap::resolve(&Isin, UnixNano)` 는 시각 의존 + conf 의존이라 OMS 쪽이 맞다.
feed 는 이미 **raw ISIN allow-set** 으로 거르고 있어 손댈 게 없다.

두 가지 주의:

1. **resolve 입력 시각은 레코드의 `recv_ns`** 다. OMS 가 읽는 시점의 `now()` 를 쓰면
   롤 경계를 걸친 메시지가 새 근월물로 잘못 붙는다.
2. `resolve()` 의 passthrough 경로는 `InstrumentId::from_bytes` → `from_string` →
   **`CACHE.lock()`** 이다. **구독 ISIN 집합을 기동 시 전부 미리 intern** 해서
   런타임에 인터너를 안 건드리게 한다.

### 시각

`utilities::timer::get_unix_nano()` 를 쓴다 — 내용은 `SystemTime::now()` 이고,
2026-08-29 에 TSC 기반 시계가 드리프트해서 갈아탄 결정이다. 프로젝트의 유일 sanctioned
`std::time` 사이트이므로 raw `std::time` 을 새로 쓰지 않는다.

- Windows 에서 `GetSystemTimePreciseAsFileTime` → QPC 보간, ~20-30ns. 속도 문제 없음.
- **단조가 아니다.** `UnixNano` 가 u64 라 `a - b` 가 랩한다. 시각 차는 전부 `saturating_sub`.
- **순서의 권위는 `producer_seq`**, `recv_ns` 는 측정·라벨링용.
- `venue_ns` 는 **조립된 절대 ns** 를 싣는다. KRX 원문 `매매처리시각` 은 `HHMMSSuuuuuu` 로
  날짜가 없어 자정에 랩한다 — 그 문제는 feed handler 안에서 끝낸다.

> 이 머신 시계 상태(2026-09-12): 루트 분산 **75.5 ms**, 폴 간격 **16384s(4.5시간)**,
> 소스 time.windows.com. 해상도는 119ns 인데 **절대 정확도는 수십 ms** 다.
> `recv_ns` 를 `venue_ns` 와 빼서 피드 지연을 재려면 w32time 부터 조여야 한다
> (가까운 소스 + 짧은 폴, 예: `time.kriss.re.kr`).

## 7. seq 두 종류

| | 잡는 사고 | 어디에 |
|---|---|---|
| **`producer_seq`** | feed→OMS **링**에서 증발 | 레코드 헤더 |
| **거래소 seq** | 네트워크 유실 | **handler 내부** (레코드에 싣지 않음) |

`producer_seq` 는 feed handler 가 링에 쓸 때마다 1씩 올린다. 현재 `src/engine/data.rs:143`
의 `let _ = producer.push(msg)` 처럼 가득 차면 조용히 버리는 유실을 보이게 한다.

```
if rec.producer_seq != expect { dropped += rec.producer_seq - expect; }
expect = rec.producer_seq + 1;
```

반응은 `kind` 로 갈린다 — **스냅샷 계열은 다음 스냅샷이 고치고**(카운터만),
**델타 계열은 복구 트리거 필수**(`src/data/recovery/`).

## 8. 플래그 — 결론만 싣고 근거는 안 싣는다

**OMS 가 `venue` 로 분기해야 읽히는 비트는 공통 필드가 아니다.**

| 비트 | OMS 동작 |
|---|---|
| `VENUE_TIME_VALID` | `venue_ns` 를 쓸지 말지 |
| `STALE` | 이 데이터로 판단하지 않음 |
| `BID_EMPTY` / `ASK_EMPTY` | 한쪽 북 처리 |

`STALE` 이 핵심이다. **판단 방법은 handler 마다 다르지만 결론은 하나다** —
KRX 는 매매처리시각 나이, FIX 는 하트비트 타임아웃·로그아웃, 선물/현물 레그는 레그 나이
(`fut_stale_seconds`, `max_leg_age_seconds`). OMS 는 어떻게 판단했는지 알 필요가 없고,
알게 되면 OMS 가 handler 별 규칙을 갖게 된다.

**`VENUE_SEQ_GAP` 은 넣지 않는다** — KRX 에선 "무시해도 됨", FIX 에선 "resend 대상"이라
한 비트에 두 의미가 들어간다. 거래소 seq 검사·resend·하트비트 주기·유실 카운터·A/B dedup 은
전부 handler 내부에 남는다.

`RECOVERED` 는 델타 피드를 붙일 때 추가한다. 지금은 불필요.

이 규칙의 실익: **handler 를 추가할 때 와이어를 안 건드린다.**

## 9. 장애 감지 — "안 꺼지는" 쪽이 위험하다

죽는 건 `boot_id` 로 잡히지만 안 죽는 건 아무 신호가 없다.

| 상황 | 증상 | 탐지 |
|---|---|---|
| A. 살아있는데 데이터 없음 | `producer_seq` 정지 | **Heartbeat 레코드** — "조용한 시장"과 구분 |
| B. 프로세스 행 | heartbeat 도 정지 | 위와 같음. **단 조건 있음** ↓ |
| C. 데이터는 오는데 내용이 stale | seq 증가, heartbeat 정상 | `recv_ns - venue_ns` 나이 → `STALE` |
| D. 죽었는데 shm 이 남음 | 옛 데이터 반복 | heartbeat, 또는 프로세스 핸들 `WaitForSingleObject(h, 0)` |

**heartbeat 는 반드시 수신 루프와 같은 스레드에서 찍는다.** 별도 watchdog 스레드로 빼면
정작 수신 루프가 멈춰도 heartbeat 가 계속 나와 감시가 아니라 위장이 된다.
`KrxUdpReceiver::receive` 가 non-blocking busy-spin 이라 루프 안에 넣기 쉽다.

**C 는 이미 난 사고다** — "conf 에 stale 가드 누락(기본 0 = 꺼짐)이 20분 정지 중 얼어붙은
북 호가의 원인". **stale 문턱 기본값을 꺼짐으로 두지 말 것.** conf 에 없으면 보수적인 값이
들어가고, 끄려면 명시적으로 끄게 한다.

권장 초기값: heartbeat 발신 100ms / OMS 판정 500ms (둘 다 conf).

> 미결: 피드 사망 판정 시 **기존 포지션 청산을 허용할지**. 시장이 안 보이는 상태의 청산이
> 더 위험할 수 있다. `resume_hold_seconds` 가 "청산까지 막아야 함"으로 되어 있는 것과 같은
> 성격의 판단.

## 10. KRX 와 FIX 는 성격이 정반대다

| | KRX (UDP 멀티캐스트) | FIX (TCP 세션) |
|---|---|---|
| 유실 | 상시. 스냅샷이 고침 | TCP 가 막음. 나면 세션 사고 |
| 역전 | 가능 (실측 0, §11) | 없음 |
| seq 성격 | 종목 × 보드, **공백일 수 있음** | 세션 전용, 항상 있음 |
| 갭 대응 | 무시 | **resend 요청** |
| 조용할 때 | 유실과 구분 불가 → heartbeat 필요 | `35=0` 하트비트 |

**공통 코드로 묶지 말 것.** KRX 에서 seq 검사를 뺀다고 FIX 에서도 빼면 안 된다.

### KRX: venue seq 를 쓰지 않는다

1. 5단 전체 스냅샷이라 유실이 자가치유된다
2. 역전 실측 0 (§11)
3. 2026-04-01 까지 필드가 공백이라 백테스트 구간과 라이브가 다른 규칙으로 돈다
4. UDP 에서 유실은 예외가 아니라 상시 조건

대신 **단조성 가드만 둔다** — 종목별 `last_venue_ns` 보다 오래된 레코드는 버린다.
비용은 i64 비교 하나. 역전은 자가치유가 아니라서(N+1 뒤에 N 을 적용하면 옛 북을 현재로
설치한다) 싸게 한 줄 둘 가치가 있다.

## 11. 측정된 사실 (2026-09-12, `E:/Data/krx_pcap`)

B606F = IFMSRPD0034 파생 우선호가 5단, A306F = IFMSRPD0036 파생 체결. 공통헤더:

```
[0:2] 데이터구분값 "B6"   [13:15] 보드ID "G1"    [17:29] ISIN "KR4A65660008"
[2:5] 정보구분값  "06F"   [15:17] 세션ID "10"    [35:47] 매매처리시각 "083000012394"
[5:13] 정보분배일련번호(8)                        [323]   종료키워드 0xFF
```

원시 바이트가 `quote.rs:17-29` 의 레이아웃 주석과 정확히 일치한다(43,762건 스캔, 종료키워드 불일치 0).

**① 정보분배일련번호는 (종목 × 보드) 단위다.** 표준서:
*"시세 : 종목별 보드별 부여 (※ 대용량 서비스에서 제공)"*.
채널 카운터가 아니므로 필터 뒤에서 검사해도 가짜 갭이 안 생기고, 반대로
**liveness 신호로는 못 쓴다**(종목이 조용하면 seq 도 멈춘다).

**② 우리 회선에서 이 필드는 2026-04-01 까지 공백이었다** — 표준서의 "대용량 서비스에서
제공" 단서 그대로.

| 날짜 | B606F seq |
|---|---|
| 03-24 / 03-27 / 03-31 / 04-01 | **100% 공백**(스페이스 8바이트) — 04-01 은 15,811건 전량 |
| 05-21 이후 전부 | 채워짐 (종목별 `00000001` 부터, 중복 0) |

캡처가 04-02~05-20 비어 있어 전환 시점은 **2026-04-01 ~ 05-21 사이**로만 좁혀진다.
공백을 0 으로 파싱하면 매 메시지가 갭으로 보이므로, 공백은 "갭 없음"이 아니라
**"측정 불가"** 로 구분해야 한다.

**③ 캡처에 12% 유실이 있다.** 20260731, (종목, 보드)별:

```
KR4A75680004/G1 : seq 1~7154 중 6230건 관측 · 구멍 486곳 · 누락 924건 (12.9%)
KR4A67690003/G1 : 누락 901건 (12.6%)
KR4A75690003/G1 : 누락 176건 (2.5%)
```

스캔 범위를 1.2GB → 2.6GB 로 2배 늘려도 같은 구간 숫자가 한 건도 안 바뀌어 스캔 아티팩트가
아니다. 중복 0건이라 A/B 이중선도 아니고, `1070~1089` 같은 **20건 연속** 패턴이라
수신 버퍼 오버플로로 보인다. KRX 가 번호를 건너뛴 게 아니라 **우리가 못 받은 것**이다.

→ `krx_parquet_db` 에도 같은 구멍이 있다. 다만 5단 전체 스냅샷이라 북이 **깨지는 게 아니라
늦는다**. 호가 수명 0.3~0.5초를 다투는 메이커 라인에는 영향이 있을 수 있다.
**03~04월 구간은 seq 가 공백이라 측정 자체가 불가능하다.**

**④ 역전은 0건.** 20260731 B606F 43,762건, pcap 도착 순서 기준:

```
인접쌍 43,740개 중  seq 역전 0건 · 매매처리시각 역전 0건 · 동률 0건
매매처리시각: 공백 0건 · 비숫자 0건 / 43,762건     ← seq 와 달리 항상 채워짐
(종목,보드) 22개 전부 seq 정렬 = 시각 정렬
```

유실은 순서를 바꾸지 않고 그냥 없앤다. 관측되지 않았을 뿐 불가능한 건 아니다
(A/B 이중선, 경로 변경, 재전송 포트 20001 혼용 시 생길 수 있다).

## 12. 시험 데이터

`apps/smbs_to_fix` (bin `smbs_to_fix`) — SMBS USDKRW parquet → 가짜 FIX 캡처.
출력 `E:/Data/smbs_fix_db/{yyyymmdd}/` (122일, 1.41 GB 생성 완료).

- `usdkrw_smbs.fix` — SOH 구분 원시 바이트열
- `usdkrw_smbs.idx.csv` — `offset,len,msg_seq,msg_type,venue_unix_nano,local_unix_nano`
- `--gap-every N` — MsgSeqNum 은 소비하고 파일에선 빼서 **진짜 시퀀스 구멍**을 만든다

`52` SendingTime = venue+5ms, `272`/`273` = venue 로 나눠 넣었다 — 디코더가 `52` 가 아니라
`272/273` 에서 venue 시각을 집는지 확인용.

**인코더를 `src/` 에 두지 않은 것은 의도적이다.** 생성기의 인코더와 앞으로 쓸 디코더가
**독립 구현 2개**여야 서로를 검증한다. 공용 코드로 묶으면 태그 쓰기 로직의 버그가
양쪽에서 상쇄돼 안 보인다.

## 13. FIX 파서 선택

**MD 는 직접 짠다.** 필요한 건 프레이밍(`8=`~`10=`), 체크섬, tag-value 스캔, 반복그룹
(`268`/`269`/`270`/`271`) 뿐이고, `KrxDataParse::validate` 의 "길이 + 엔드마커" 패턴이
FIX 의 "BodyLength + CheckSum" 에 그대로 대응된다.

### 2026-09-13 구현 완료 — `src/data/fix/` (베뉴 무지) + `src/data/exchanges/smbs/` (방언)

**FIX 는 프로토콜이고 베뉴가 아니다.** 파서를 `exchanges/smbs/fix/` 아래 두려다 접었다 —
SMBS 는 FIX 를 쓰는 한 곳일 뿐이고, 두 번째 FIX 베뉴가 오면 프로토콜 코드를 복사하거나
베뉴 하나를 특별대우하게 된다. 그래서 `wire/` 가 `data/common/` 에 있는 것처럼 프로토콜 층은
`data/fix/` 에 두고, 베뉴가 아는 것만 `data/exchanges/smbs/` 로 뺐다.

```
[소켓 바이트]
     ↓ frame          fix/frame.rs        8=…9=len … 10=ddd, BodyLength+CheckSum 먼저 검사
     ↓ scan           fix/tagvalue.rs     tag=value 이터레이터 · 정수 · 스케일 소수 · UTC 시각
     ↓ decode         fix/market_data.rs  35=W / 35=X → MdMessage (스택 고정 크기, 엔트리 32)
     ↓                fix/session.rs      34 갭 · 35=0/1/2/3/4/5/A · 침묵 감시
[MdMessage]
     ↓ adapt          exchanges/smbs/market_data.rs → WireRecord(Venue::Smbs)
```

- **무할당·무복사.** `Frame`/`Field` 는 수신 버퍼를 빌리고 `MdMessage` 는 스택 값이다.
  `FrameBuffer<N>` 이 스트림을 프레임으로 끊는다(경계가 어디 떨어져도 재조립).
- **스케일은 파스 지점에서 정수화**(`FixScales`). 리더가 f64→정수를 하는 것과 같은 이유로
  반올림 지점이 아래로 새지 않는다. 스케일이 못 담는 소수는 **반올림이 아니라 에러**
  (`FixError::Precision`) — 설정 버그가 가격으로 위장하면 안 된다.
- **venue 시각은 `272`+`273`**, `52`(SendingTime)는 상대 시계라 측정용으로만 싣는다.
- SMBS 방언은 셋뿐이다: 식별자가 심볼(`wire_isin` = 심볼 좌정렬 공백채움 12바이트 —
  Jeed v2 에서는 24바이트 `NUL` 채움,
  backtest `SmbsFeedSource::wire_isin` 도 이 함수를 부른다), 호가에 수량이 없어
  `SmbsMdConfig::default_quote_size` 로 채움, 체결에 공격자 방향 없음(`trade_kind::NONE`).
- **35=X 안의 호가 갱신은 거부한다**(`SmbsMdError::IncrementalBook`) — 와이어에 delta
  레코드가 아직 없으므로(§8 `SnapshotDelta` 예약) 조용히 버리면 OMS 북이 틀린다.

  > **Jeed v2 에는 delta 레코드가 생겼다.** 그래도 거부는 그대로 둔다 — 실을 곳이 생긴 것과
  > SMBS 의 35=X 가 실제로 어떤 갱신을 보내는지 아는 것은 다른 문제이고, FIX 베뉴 어댑터는
  > 아직 없다(`documents/todo.md` §3). 크립토의 갱신ID 사슬에 해당하는 것이 SMBS 에 무엇인지
  > 확인되기 전까지 거부가 맞다.

검증: `tests/data/fix/` + `tests/data/exchanges/smbs/` **64건**. 그중 `capture.rs` 는
`E:/Data/smbs_fix_db/20260202` 를 통째로 돌려 생성기 인덱스와 맞춘다 —
**57,221 메시지(W 42,725 · X 12,457 · HB 2,039) 전부 offset·len·34·35·272/273 일치, 55,182
와이어 레코드 전부 `validate()` 통과, 시퀀스 갭 0**. 한쪽만 있는 개장 북 1건도 그대로 살아
`BID_EMPTY` 로 나온다. 캡처가 없는 머신에서는 건너뛴다.

- `../ferrumfix` 로컬 스냅샷은 **라이브러리 소스가 없다** — tracked 173개 중 `crates/` 0개,
  `.rs` 14개 전부 examples/tests. 빌드 자체가 안 된다. upstream 도 0.7.0 pre-1.0.
- `quickfix-rs` 는 실물이고 **Windows 된다**. CI 를 끈 건 고장이 아니라 `c1e9f5a`
  *"Do not have time to support CI with them"* 이고, `build.rs` 에 `TargetOs::Windows`
  분기가 있다. **스레딩도 반대 근거가 아니다** — `FixSocketServerKind::SingleThreaded` +
  `ConnectionHandler::poll()` 로 내 코어 핀 스레드에서 콜백을 받을 수 있다.
  남는 비용은 메시지당 C++ `std::string`/`std::map` 할당과 CMake/C++17 빌드 의존성.
- 프로세스를 분리하면 그 비용이 격리되므로 **주문 세션에는 quickfix-rs 가 후보로 남는다.**

## 14. Windows 코어 격리

`isolcpus`/`nohz_full`/`rcu_nocbs` 대응물이 없고 타이머 틱도 못 끈다.
`SetThreadAffinityMask`(= `core_affinity`)는 핀만 하고 **배제는 안 된다.**

실효가 큰 건 장치별 레지스트리 `Interrupt Management\Affinity Policy`
(`DevicePolicy`, `AssignmentSetOverride`)로 **DPC/ISR steering**. 측정은 LatencyMon.

이 머신 현재 상태(2026-09-12): 전원 구성표 "균형 조정" + `PROCTHROTTLEMIN=0`,
VBS+HVCI 실행 중(하이퍼바이저 위), SMT 켜짐(= 격리 코어 1개 = 논리 2개).

> 위 측정은 Ryzen 7 7700(8코어/CCD 1장) 에서 한 것이다. 타겟은 **Ryzen 9 9950X**
> (16코어/CCD 2장)로 바뀌었고, 지터 수치는 다시 재야 한다. 바뀐 제약은 §2·§3.1 —
> L3 가 CCD 당 32 MiB 로 갈리므로 생산자와 그 소비자를 같은 CCD 에 핀한다.

**다만 주문 경로가 ms 단위라 OS 지터(µs)는 3자릿수 아래 항이다.** 목표는 sub-µs 가 아니라
**"멀티캐스트 패킷을 안 떨어뜨리는 것"** — 그 기준이면 Windows 로 충분하다.
진짜 sub-µs 가 필요해지면 그건 튜닝이 아니라 Linux + 커널바이패스(onload) 이주 문제고,
`KrxUdpReceiver` 주석의 onload 언급이 이미 그 전제다. Windows 전용 API 는 얇은 레이어
뒤에 숨겨 두는 정도가 보험으로 적당하다.

**보험은 2026-09-13 에 찾았다.** `jeed-shm`·`jeed-krx` 가 리눅스에서 돈다 (WSL 전체 통과,
clippy 0). 갈린 건 `unsafe extern` 이 모인 두 디렉터리(`mapping/`, `recv/socket/`)뿐이고
그 위층엔 `#[cfg]` 이 없다. 즉 **이주는 이제 튜닝 문제지 포팅 문제가 아니다** — 남는 일은
코어 격리와 NIC 쪽이고, 그건 리눅스가 원래 잘하는 것들이다.

## 15. 미결

- [x] 와이어 레코드 `#[repr(C)]` 타입 정의 — `src/data/common/wire/` (2026-09-13; 레코드·세그먼트 헤더·어댑터, 테스트 `tests/data/common/wire/` 56건). 크기는 §5 참고
- [x] backtest 리플레이를 와이어 레코드 생산자로 전환 + 기준점 A/B (2026-09-13; `read_krx_day` → `Vec<WireRecord>`, 피드는 `wire::adapt::fill_*` 경유, 캐시 `fe.feed/3`. A/B: KODEX 레버리지 v1 2창(w0601 4,296만 · w0615 4,938만)과 USDKRW 메이커 v2 w0601 이 summary·daily·fills **바이트 단위 동일**. 새 캐시는 옛 f64 포맷보다 14~20% 작다)
- [x] FIX MD 파서 (2026-09-13; `src/data/fix/` {frame, tagvalue, market_data, session, error} + `src/data/exchanges/smbs/` {mod, market_data}, 테스트 64건. 실캡처 20260202 57,221 메시지 인덱스 대조 통과. 주문 세션은 별도 quickfix-rs, §13)
- [x] shm 링 구현 (2026-09-13; `crates/jeed-shm` — Windows named mapping + POSIX `shm_open`,
      덮어쓰기 링 + 슬롯 seqlock). **SPSC ×2 가 아니라 피드당 링 하나에 리더 N개로 갔다**,
      근거는 §3
- [ ] 피드 사망 시 청산 허용 여부 (§9)
- [ ] KRX 일련번호 전환일 특정 — 04-02~05-20 캡처 부재
- [ ] DB 전체 유실률 정량화 (05-21 이후 ~40일) → 백테스트 신뢰구간
