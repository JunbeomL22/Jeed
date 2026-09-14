# Jeed — 할 일

Jeed = **J**unbeom f**eed** handler. fractal-engine 의 시세 수신부를 **별도 프로세스**로 떼어내
정규화 이전의 **와이어 레코드**까지만 만들어 공유 메모리로 넘긴다. 2026-09-13 설계 시작.

설계 근거는 [feed_handler.md](feed_handler.md) 에 있고, 이 문서는 그 설계를 Jeed 저장소에서
실행하기 위한 작업 목록이다. 절 번호(§)는 전부 `feed_handler.md` 를 가리킨다.

**지금은 fractal-engine 을 건드리지 않는다.** Jeed 가 먼저 완성되고, fractal-engine 이 나중에
여기에 맞춰 자기 쪽을 고친다. 이 문서에 fractal-engine 작업 항목은 없다.

## 1. 범위

**한다**

- KRX UDP 멀티캐스트 수신 + 전문 디코딩 → `WireRecord`
- FIX 4.4 **프로토콜 층** (프레이밍 / tag-value / 35=W·X / 세션) → `MdMessage`
- shm SPSC 링 producer, 그리고 소비자가 쓸 consumer
- 와이어 레코드 ABI 정의와 버저닝

**안 한다**

- **주문.** `src/data/exchanges/krx/fep/hoa/` (KRX FEP/HOA, EXTURE 3.0) 는 범위 밖이다.
  KRX 코드의 60% 가 여기지만 Jeed 는 정보 수신 전용이다.
- **북.** 피드 핸들러는 무상태다. 디코드 + 필터 + 정규화만 하고 북을 안 갖는다 (§1).
- **정규화.** `SnapshotData` / `TradeData` 로의 변환(`wire::adapt::fill_*`)과
  `AliasMap` / `InstrumentId` 해석은 소비자(fractal-engine) 몫이다 (§4, §6).
- **fractal-engine 수정.** 완성 후 그쪽이 맞춘다.
- **FIX 베뉴 방언.** SMBS 어댑터는 실물 확인 후로 미룬다.

## 2. 크레이트 구조

```
jeed/
├─ Cargo.toml            [workspace]
└─ crates/
   ├─ jeed-wire/         ABI. 의존성 0
   ├─ jeed-shm/          named mapping(Win32) / shm_open(POSIX) + SPSC 링 (producer/consumer)
   ├─ jeed-krx/          UDP 수신 + 전문 디코더 → WireRecord
   ├─ jeed-fix/          FIX 프로토콜 → MdMessage → WireRecord
   ├─ jeed-crypto/       크립토 WebSocket JSON → WireRecord
   └─ jeed/              바이너리 (krx, fix, …)
```

```
jeed-wire ←── jeed-shm ←──┬── jeed (bin)
    ↑                     │
    ├── jeed-krx ─────────┤
    ├── jeed-fix ─────────┤
    └── jeed-crypto ──────┘
    ↑
    └──────────────────── 소비자(fractal-engine) 는 jeed-wire + jeed-shm 만
```

**`jeed-wire` 를 떼는 이유가 핵심이다.** 소비자가 KRX 디코더에 의존하면 파싱 버그 하나에
OMS 를 재배포해야 하고, 그러면 프로세스를 나눈 이유가 없다. `WIRE_FORMAT_VERSION` 을 올리는
것은 **양쪽 바이너리 동시 배포 이벤트**이므로 그 사건이 한 크레이트의 커밋 로그에만 찍혀야 한다.
backtest parquet 리플레이도 `WireRecord` 를 만드는데 그쪽엔 UDP 코드가 필요 없다.

- `Venue` / `Symbol` / `Scale` 은 **헤더 필드 자체**라 `jeed-wire` 에 둔다. `jeed-types` 를 따로 만들지 않는다.
  KRX 의 `Isin`(12B)은 반대로 `jeed-krx` 쪽이다 — 전문 레이아웃의 폭이지 와이어의 폭이 아니라서,
  섞어 두면 크립토 심볼이 길어질 때마다 KRX 오프셋이 움직인다.
- KRX 의 ASCII 고정폭 extractor 는 `jeed-krx` 안에 둔다. FIX 는 tag-value 라 안 쓴다 —
  공용으로 빼면 소비자가 하나뿐인 크레이트가 생긴다.
- `jeed-shm` 분리 이유: unsafe 와 OS API 가 여기만 모인다. 그리고 wire 는 파일·parquet
  경로에서도 쓰이는데 거긴 shm 이 필요 없다.
- **핸들러 lib 은 `jeed-shm` 에 의존하지 않는다.** 싱크 트레이트(`fn publish(&mut self, &WireRecord)`)에
  쓰고 바이너리가 shm 링에 연결한다. 테스트는 `Vec<WireRecord>` 싱크로 돈다.
- 바이너리는 `jeed` 한 크레이트에 피드마다 `[[bin]]`. conf 로딩·코어 핀·세그먼트 생성·시그널·
  리포트가 세 핸들러에서 동일하므로 lib 절반에 한 번만 쓰고, `src/bin/*.rs` 는 피드마다 다른
  몇십 줄이다. **`jeed-krx`·`jeed-crypto` 가 있다 (2026-09-13, §6).** fix 는 어댑터가 없어 아직.
- **외부 의존은 `rustls`(+`webpki-roots`) 하나다** (2026-09-13). `wss://` 가 TLS 라 피할 수 없고,
  `jeed-crypto` 의 `recv` 피처(기본 켜짐) 뒤에 있어서 디코더만 쓰는 쪽은 `default-features = false`
  로 TLS 를 링크하지 않는다. WebSocket 프레이밍은 직접 썼다 — §6 "`jeed-crypto` 수신부".

## 3. 이관 대상

출처는 전부 `../fractal-engine`. **복사해 오는 것이지 그쪽을 고치는 게 아니다.**

| 원본 | 행선지 | 비고 |
|---|---|---|
| `src/data/common/wire/{record,header,kind,payload,segment,error}.rs` | `jeed-wire` | 그대로 |
| `src/data/common/wire/adapt.rs` | **안 가져옴** | `AliasMap`·`InstrumentId` 필요 → 소비자 쪽 |
| `src/data/exchanges/krx/decode/**` | `jeed-krx` | 출력 타입 변경 필요 (§4) |
| `src/data/exchanges/krx/{enums,market_state,session}.rs` | `jeed-krx` | |
| `src/data/receiver/krx_udp.rs` | `jeed-krx` | 재작성 (§5) |
| `src/utilities/converters/extractor/**` | `jeed-krx` | `EXTRACTOR_CONTAINER` — 디코더가 필드 폭·스케일을 여기서 얻음 |
| `src/data/fix/{frame,tagvalue,market_data,session,error}.rs` | `jeed-fix` | 그대로 |
| `src/data/exchanges/binance/json.rs` | `jeed-crypto/src/json.rs` | 스캐너는 그대로, 레벨 파서만 고정배열로 |
| `src/data/exchanges/binance/decode/**` | `jeed-crypto/src/binance/` | 출력 타입 교체 + 디코더당 struct → `Instrument` + 자유함수 |
| `src/data/receiver/crypto.rs` | `jeed-crypto/src/recv/` | 완료. epoll/thread-per-socket 두 갈래는 안 가져왔다 — 수신 루프 하나가 연결 하나다(§6). **원본은 2026-09-14 삭제** |
| `src/data/exchanges/{upbit,bithumb,okx,bybit}/decode/**` | `jeed-crypto/{upbit,bithumb,okx,bybit}/` | 완료. 시퀀스 추적(`last_seq_id`)은 안 가져왔다 — 핸들러는 무상태 |
| `src/data/exchanges/{bitget,gate,kucoin}/decode/**` | `jeed-crypto/{bitget,gate,kucoin}/` | 완료. kucoin 은 spot/futures 두 모듈 |
| `src/data/exchanges/{htx,kraken}/decode/**` | **안 가져옴** | 결정(2026-09-13). htx 는 gzip 프레이밍, kraken 은 객체형 레벨·RFC3339·시퀀스 없음 |
| `src/data/exchanges/*/encode/**` | **안 가져옴** | 주문 |
| `src/data/recovery/**` | **안 가져옴** | 델타 복구는 북을 가진 소비자 몫. 경계 확정 (2026-09-14) → §17 |
| `src/data/exchanges/smbs/**` | 보류 | 베뉴 방언, 실물 확인 후 |
| `src/data/exchanges/krx/fep/**` | **안 가져옴** | 주문 |

## 4. 이관의 실체는 "복사"가 아니라 "출력 타입 교체"

지금 디코더는 원시 바이트에서 **`SnapshotData` 로 직행**한다:

```rust
// fractal-engine src/data/exchanges/krx/decode/derivative/quote.rs:120
pub fn decode(&self, system_time, payload, quote_level_map, id_alias,
              buffer: &mut SnapshotData) -> Result<bool, DataError>
```

`feed_handler.md` §4 그림은 `live: KRX 디코더 → WireRecord` 로 그려져 있지만 §15 체크박스는
**backtest 리플레이만** 전환 완료다. Jeed 작업의 본체는 이것이다:

```
지금:  payload ─▶ decode(…, &mut SnapshotData)     ← AliasMap · QuoteLevelMap · InstrumentId 필요
Jeed:  payload ─▶ decode(…, &mut WireRecord)       ← raw ISIN 만, 맵 불필요
```

그 대가로 `AliasMap` / `QuoteLevelMap` / `InstrumentId` 의존이 통째로 떨어진다.
`src/data/` 의 `crate::types` 참조 46 건 중 상당수가 여기서 사라진다.

남는 crate-내부 의존(이관 시 해결):

| 의존 | 건수 | 처리 |
|---|---|---|
| `crate::types` | 46 | 값 타입만 `jeed-wire` 로. `InstrumentId`·`AliasMap` 은 안 넘어감 |
| `crate::utilities::converters` | 27 | `Scale` → `jeed-wire`, extractor → `jeed-krx` |
| `crate::types::order` | 17 | 전부 FEP/주문 쪽 → 안 가져옴 |
| `crate::{engine,instrument,backtest}` | 5 | 끊음 |

### 검증 기준점

fractal-engine 을 안 고치더라도 **기존 디코더는 유일한 참조 구현**이다. 정확성 검증은
프로덕션 의존이 아니라 **일회성 대조**로 한다: `E:/Data/krx_pcap` 의 같은 바이트를
(구) `SnapshotData` 경로와 (신) `WireRecord` 경로에 각각 넣고 값이 같은지 본다.
표준서 레이아웃만 보고 짠 디코더는 표준서를 잘못 읽으면 조용히 틀린다.

## 5. 수신 구조

> ⚠️ **2026-09-13 표준서 확인으로 초안이 뒤집혔다.** "trcode 하나 = 멀티캐스트 그룹 하나 =
> 소켓 하나" 를 전제로 소켓↔디코더 정적 결합을 설계했으나, 실제로는 **한 포트에 데이터구분이
> 전부 섞여 온다.** 아래가 표준서 기준의 수정안이다.

### 확정된 채널 구조 (접속표준서 UDP 공통정보 v1.26 / 정보분배 v1.341)

**trcode 5바이트 = `[데이터구분 2][정보구분+시장구분 3]`.** 뒤 3자리가 상품군이다:
`01F` 코스피200선물 · `02F` 코스닥150선물 · `03F` 코스피200옵션 · `04F` 주식선물 ·
`05F` 주식옵션 · `06F` 금융상품선물(국채·금리·통화) · `08F` 변동성지수선물 · `11F` 미니코스피200선물 ·
`16F` 코스피200위클리옵션 / `01S` 주식 · `02S` ELW · `03S` ETF · `04S` ETN.

관심 데이터구분:

| 데이터구분 | 인터페이스 | 내용 | 길이 | trcode |
|---|---|---|---|---|
| `B6` | IFMSRPD0034 | 파생 우선호가 5단 | 324 | `B601F`~`B603F`, `B606F`~`B616F` (+`B614F` 파생B) |
| `B6` | IFMSRPD0035 | 파생 우선호가 10단 (주식파생) | 554 | `B604F`, `B605F` |
| `A3` | IFMSRPD0036 | 파생 체결 | 173 | `A301F`~`A316F` |
| `G7` | IFMSRPD0037 | 파생 **체결+우선호가 5단** | 431 | `G701F`~`G703F`, `G706F`~`G716F` (+`G714F` 파생B) |
| `G7` | IFMSRPD0038 | 파생 체결+우선호가 10단 (주식파생) | 661 | `G704F`, `G705F` |
| `B7` | IFMSRPD0003 | 증권 우선호가 **MM/LP호가 포함** | 830 | `B702S` ELW, **`B703S` ETF**, `B704S` ETN |
| `V1` | IFMSRPD0043 | 파생 가격제한폭 확대 발동 | 65 | `V101F`~`V116F` |
| `Q2` | IFMSRPD0042 | 파생 **동적상하한가** 적용 및 해제 | 65 | `Q201F`~`Q216F` |
| `M4` | IFMSRPD0019 | 장운영 스케줄 공개 | 83 | `M401F`~`M413F` |
| `A0` | — | 파생 종목정보(마스터) | | `A001F`~`A013F` |

### 종목 상태의 세 축 — 서로 다른 것이다

| 축 | 전문 | 값 | 성격 | 현행 |
|---|---|---|---|---|
| **가격제한폭** | `V1` IFMSRPD0043 | 상한가·하한가 + **상한단계·하한단계** | 하루짜리 범위, 단계적으로 **확대**. 상·하한 단계 독립 | fractal-engine 구현됨 |
| **동적상하한가** | `Q2` IFMSRPD0042 | 동적상한가·동적하한가 + **설정코드**(0 해제/1 설정/2 재설정) | 장중에 켜졌다 꺼졌다. 직전가 근처의 좁은 밴드 | **미구현 (신규)** |
| **CB·세션** | `M4` IFMSRPD0019 | 장운영 스케줄·정지 | 시장 단위 | fractal-engine 구현됨 |

두 전문의 구조 차이가 성격을 그대로 보여준다. **V1 엔 단계(段階)가 있다** — 제한폭은 1→2→3 단계로
넓어지는 것이라 "지금 몇 단계인지"가 상태다(`price_limit_processing.md`: "V1 상한 단계와 하한 단계는
독립적으로 보관한다"). **Q2 엔 설정코드가 있다** — 켜짐/꺼짐이라 단계 개념이 없다.

둘은 **동시에 걸린다.** 가격제한폭이 바깥 울타리(하루 범위), 동적상하한가가 안쪽 울타리(직전 체결가
± α)고 주문 가능 범위는 **둘의 교집합**이다. 주식파생은 1단계 가격제한 범위 안에서만 적용된다.
`CB_가격제한폭.md` 가 "가격제한폭 확대는 CB 가 아니다" 라고 구분한 것처럼 **동적상하한가는 그 둘
어느 쪽도 아니다.**

제도 내용은 [krx/실시간가격제한.md](krx/실시간가격제한.md) 에 따로 있다. 요점 셋:

- 밴드 밖 호가는 **거래소가 접수 자체를 거부**한다 (VI 처럼 매매방식을 바꾸는 게 아니다)
- **해제되면 지정가·조건부지정가만 가능**해진다 — 해제는 값의 변화가 아니라 **주문 가능 종류의
  변화**라 `G7` 인라인 값만 봐서는 알 수 없다. 그래서 `Q2` 를 따로 받아야 한다
- **적용 제외 종목이 있다**(원월물·선물스프레드·주식옵션·돈육선물). 세션마다 폭도 다르다
  (야간은 정규의 2배) — 보드ID·세션ID 를 같이 봐야 한다

`Q2` 의 ISIN 은 `V1` 과 같은 `15..27` 이다(둘 다 보드ID 다음이 바로 종목코드). `krx_udp.rs` 의
`payload_isin` 이 `V1 => 15..27` 로 처리하는 그 분기를 Q2 도 탄다.

**복구 가능성이 갈린다.** `Q2` 의 값은 `G7`(IFMSRPD0037) 오프셋 154/163 에 **매 메시지 인라인으로**
실린다 — 재접속해도 다음 체결 한 번이면 현재 밴드가 들어온다. `V1` 은 그런 동반이 없어
최초 수신 전까지 미확인이고, 그래서 §7 의 "`A004F` 마스터에서 장 시작 제한가격 초기화" 숙제는
**V1 쪽에만** 필요하다.

**소켓 = (멀티캐스트 그룹IP, 포트).** 파생 시세는 포트가 **상품군별**로, 증권은
**정보분배그룹번호별**로 갈린다. 재전송 포트는 파생 20301 / 증권A 20001 / 증권C 20201.

```
파생A 통합선물100M   233.38.231.92
  10302 코스피200선물   10303 코스닥150선물   10304 개별주식선물
  10305 금융상품선물     10306 일반상품선물     10307 미니코스피200선물
  10308 변동성지수선물   10309 섹터지수선물     10310 KRX300선물
  → 포트 하나로 A7 · O6 · B6 · A3 · G7 · R1 · A6 · Q2 · V1 · I2000 이 전부 섞여 온다

파생A 콜옵션100M     233.38.231.96   10322 코스피200옵션콜 … 10327 위클리콜   (풋은 .97)
파생A                233.38.231.93   10315 → M4 장운영스케줄 · IF 그룹호가접수중지  ← 여기만 옴
파생A                233.38.231.91   10301 → A0 종목정보 · C4 협의거래 · H1/H2/H3 통계 · B2 Snapshot
증권C 증권상품100M   233.38.231.80   10245~10249 (분배그룹 00006~00010)
  → B702S/B703S/B704S(ELW/ETF/ETN) + B803S/B804S 단일가 + IE03S/IE04S 경쟁대량 이 한 포트에
```

### 결과 1 — 정적 디스패치 불가, trcode 필터는 진짜 필터

소켓 하나에 데이터구분이 여럿이므로 소켓에 디코더를 미리 붙일 수 없다. 런타임에
`payload[0..5]` 로 분기한다. 기존 `KrxUdpReceiver::trcode_filter` 는 **설정 오류 탐지가 아니라
진짜 필터로 남는다** — ETF 만 원해도 ELW/ETN 이 같이 오고, 호가만 원해도 배분정보가 같이 온다.

`isincode_filter` 도 남는다. 상품군 포트에는 그 상품군 전 종목(옵션이면 전 행사가)이 실린다.

| 필터 층 | 어디서 | 단위 | 비용 |
|---|---|---|---|
| IGMP 그룹 가입 | conf → 소켓 | 상품군 / 분배그룹 | 0 (NIC 에 안 옴) |
| trcode allow-set | 수신 루프 | 데이터구분 | 해시 조회. **진짜 필터** |
| 길이 검증 | 디코더 | 전문 | `n == expect_len` |
| ISIN allow-set | 수신 루프 | 종목 | 해시 조회 |

### 결과 2 — cold 스레드가 대부분 없어진다

`V1`(가격제한폭 확대)은 `B6`/`A3`/`G7` 과 **같은 포트**로 온다. 소켓으로 hot/cold 를 나눌 수 없고,
나눌 이유도 없다 — 이미 받았으므로 spin 비용은 냈고 남은 건 디코드뿐인데 초당 수 건이다.

**진짜 cold 는 다른 포트로만 오는 것**뿐이다:

| | 소켓 | 실린 것 |
|---|---|---|
| hot (spin, 핀) | `.92:10302` 선물 · `.96:10322` 콜 · `.97:103xx` 풋 | **B6 G7** · (A3 V1 Q2 A7 O6 A6 R1 도 같은 포트로 오지만 hot 에서 안 건짐) |
| cold (block, 핀) | `.93:10315` · `.91:10301` (+옵션 `.95:10321`) | **M4** IF A0 C4 H1 H2 H3 B2 |

`V1`·`Q2` 는 hot 소켓으로 들어오지만 **cold 로 분류한다** — 초당 수 건이고 지연이 중요하지 않다.
같은 소켓의 메시지를 다른 링으로 보내려면 hot 스레드가 두 producer 를 갖게 되므로,
**hot 스레드가 hot 링에만 쓰고 `V1`/`Q2` 도 거기 실어 보낸다.** 분류는 소비자가 `kind` 로 한다.

코스피200 선물+옵션이면 **hot 3 소켓**. spin 한 바퀴가 짧아 여유롭다.

| | hot | cold |
|---|---|---|
| 막는 지점 | 없음 (`WouldBlock` 즉시 리턴) | `WSAPoll` 이 데이터 올 때까지 |
| CPU | 100% | ~0% |
| 지연 | 소켓 한 바퀴, µs | OS 가 깨우는 시간, 수십 µs ~ ms |
| 하트비트 | 매 사이클 | `WSAPoll` 타임아웃이 곧 틱 |
| 핀 | 필수 | **한다** — Windows 엔 `isolcpus` 가 없어(§14) hot 코어를 비워둘 강제 수단이 없다. hot 코어와 SMT 형제만 빼고 핀해서 회피 |

- **핀 코어 1개 = 수신 스레드 1개 = producer 1개 = 링 1개.** hot 을 두 코어로 쪼개면 producer 가
  둘이 되고 SPSC 라 링도 둘이 된다. §2 가 라우터를 금지했으니 합칠 방법이 없다.
  hot 한 코어가 못 빤다는 **측정**이 나오기 전까지 하나로 간다.
- cold 도 producer 라 링이 하나 더 생긴다. 인정하고 간다 — cold 는 초당 수 건이라
  커서 하나 더 도는 건 공짜다.
- 나중에 합칠 수 있다. §3 대로 락인은 링이 아니라 레코드 포맷이라 `WireRecord` 는 그대로고
  링 개수만 바뀐다.

### conf 두 파일

**① `conf/krx_trcodes.toml` — 표준서에서 생성, 손으로 안 씀**

**IP·포트는 생성하지 않는다.** 표준서의 그룹IP/포트는 표준 배정이고 실제 회선(증권A / 파생A /
파생B, 100M / 12M)에 따라 다르다. 생성기가 만들어내면 틀린 값이 조용히 박힌다.
생성기가 만드는 건 **trcode 메타데이터**뿐이다. **생성 완료 (2026-09-13,
`tools/gen_krx_trcodes.py` → `conf/krx_trcodes.toml`, 인터페이스 123 · trcode 538).**

실제 모양은 스케치보다 한 겹 정규화됐다 — 길이·이름은 인터페이스 단위로 고정이라
538번 반복할 이유가 없고, 표준서 버전이 올라갈 때 한 줄만 바뀐다:

```toml
[product_group]                 # 3바이트 → 상품군 (별첨-정보구분코드에서)
"01F" = "kospi200선물"

[data_class]                    # 2바이트 → 이 구분을 쓰는 인터페이스들 (역산)
"B6" = ["IFMSRPD0002", "IFMSRPD0023", ..., "IFMSRPD0034", "IFMSRPD0035", ...]

[interface]                     # 길이는 인터페이스 단위로 고정
IFMSRPD0037 = { length = 431, market = "DRV", period = "실시간", name = "파생 체결 + 우선호가 (우선호가 5단계)" }

[code]                          # trcode → 인터페이스
G701F = { interface = "IFMSRPD0037", group = "01F", channel = "파생A", market = "DRV" }
```

trcode 수백 개이고 길이가 틀리면 디코더가 통째로 어긋난다. 이건 기계가 만든다.

> ⚠️ **데이터구분 2바이트로는 레이아웃이 안 정해진다.** `B6` 하나가 인터페이스 8개로
> 갈리고 길이가 324~1387B 다(파생5단 324 / 파생10단 554 / 증권 590 / 채권 462 / 소액채권 882 /
> REPO 1387 / 금현물 795 / 배출권 325). 상품군(뒤 3자리)까지 봐야 정해진다.
> 디스패치 키는 **5바이트 전부**다.

> ⚠️ **trcode 가 인터페이스에 1:1 이 아니다.** 생성기가 세 갈래로 가른다:
>
> 1. **회선마다 다른 전문** — `B201S` 는 증권A 회선에서 685B(증권 Snapshot),
>    주식파생 기초자산 회선에서 **136B**. 5바이트만 보고는 못 고른다.
>    `[code_by_channel]` 로 따로 빼 놨다(`B201S`/`B201Q`/`B203S` 3건).
>    **어느 소켓에서 받았는지를 디코더에 같이 넘겨야 한다** — 소켓↔디코더가 완전히
>    무관하지는 않다는 뜻이다
> 2. **같은 전문, 다른 시각** — `H101F`(파생 상품별 투자자별 통계)는 IFMSSTD0005(장중 30초 주기)와
>    IFMSRID0007(확정치 07:30/16:30) 양쪽에 있고 길이가 같다(166B). 디코드는 하나,
>    의미만 시각으로 갈린다 → `[code]` 의 `also` 필드
> 3. **전문 안의 구분자** — `M200G / 1`(시세) vs `/ 2`(종가). 우리 시장 아님

**② `conf/krx.example.toml` — 템플릿 (2026-09-13).** `krx.toml` 로 복사해서 IP·포트를 채운다.

실제 값이 들어간 모양:

```toml
[[feed]]
name    = "hot"
mode    = "spin"
cores   = [2]                # SMT 형제 10 은 비워둠
ring    = "jeed.krx.hot"
sockets = ["...:10302", "...:10322", "...:10331"]   # 실제 회선 값을 직접 적는다
trcodes = ["B601F","G701F","V101F",
           "B603F","G703F","V103F"]                 # 받을 것만. 나머지는 버린다

[[feed]]
name    = "cold"
mode    = "block"
cores   = [4, 5, 6, 7]       # hot(2)·형제(10) 제외
ring    = "jeed.krx.cold"
sockets = ["...:10315", "...:10301"]
trcodes = ["M401F","M403F","A001F","A003F"]

[filter]
isin = "conf/isin_allow.txt"
```

`sockets` 는 가입할 (그룹IP, 포트), `trcodes` 는 그 소켓에서 **건질 것**이다. 둘이 따로인 이유가
위의 "한 포트에 섞여 온다" 다. `mode` 를 conf 에 두는 게 핵심 — "hot/cold" 라는 이름이 아니라
spin 이냐 block 이냐가 실제 동작이다.

기동 시 검증: `trcodes` 의 각 코드가 `krx_trcodes.toml` 에 있어야 한다.
**단 없다고 기동 실패는 아니다** — 표준서가 실제 회선보다 뒤처진다(§14②: `17F` 상품군 3개 코드가
회선에는 오는데 v1.341 에 없다). 없는 코드는 경고를 찍고 conf 가 명시적으로 허용한 경우에만 받는다.
소켓↔trcode 대응은 **검증할 수 없다** — 어느 포트에 뭐가 실리는지는 회선 배정이라 우리가 모른다.
대신 런타임에 "이 소켓에서 한 번도 안 본 trcode" 를 카운터로 노출해서 잘못된 배선이 보이게 한다.

## 6. 작업 순서

- [x] **워크스페이스 뼈대** (2026-09-13) — edition 2024, `crates/` 아래. 지금은 `jeed-wire` 하나
- [x] **`jeed-wire`** (2026-09-13) — types/error/kind/header/payload/record/segment, 의존성 0,
      테스트 51건, clippy·rustdoc 무경고. `deepsize` 와 `VenueMismatch`(어댑터용)는 안 가져왔다.
      레이아웃 단언(640B / align 64 / 암묵 패딩 0 / 필드 오프셋)이 컴파일 타임과 테스트 양쪽에 있다
      (576B 로 시작했다가 동적상하한가를 싣느라 640B 로 늘렸다 — §12)
- [x] **`jeed-shm`** (2026-09-13) — Windows named mapping(`CreateFileMapping(INVALID_HANDLE_VALUE,…)` +
      `MapViewOfFile`, 페이지파일 백업) 링. producer/consumer 양쪽. 테스트 44건, clippy·rustdoc 무경고.
      의존성은 `jeed-wire` 하나 — Win32 진입점 6개를 직접 선언했다(`windows-sys` 를 소비자에게
      물려주지 않으려고). 설계 결정 두 개는 아래 §11
- [x] **trcode 테이블 생성기** (2026-09-13) — `tools/gen_krx_trcodes.py`:
      인터페이스목록 v1.341 xlsx → `conf/krx_trcodes.toml` (인터페이스 123 · trcode 538
      · 회선별 3). **IP·포트는 생성하지 않았다.** `conf/krx.example.toml` 템플릿도 같이.
      파이썬인 이유: 표준서 버전이 올라갈 때만 도는 스크립트라 `calamine` 을 워크스페이스
      의존성으로 들일 값을 못 한다. 결과 TOML 이 커밋되고 그게 빌드가 보는 유일한 입력이다.
      **부수 발견 두 개는 §5 의 ⚠️** (5바이트 전부가 디스패치 키 / trcode 가 인터페이스에 1:1 아님)
- [~] **`jeed-krx` 디코더 + 출력 타입 교체** — `SnapshotData` → `WireRecord`.
  - [x] **바닥층** (2026-09-13) — `error`/`field`/`trcode`/`message`/`clock`.
        필드 리더는 전부 `Option` 을 돌려준다(**공백 ≠ 0**). 가격은 전문 안의 `.` 을 찾아
        값과 소수자리수를 같이 낸다 — 상품군으로도 안 갈리기 때문(CLAUDE.md)
  - [x] **호가 B6 (0034 324B / 0035 554B)** (2026-09-13)
  - [x] **체결+호가 G7 (0037 431B / 0038 661B)** (2026-09-13) — 동적상하한가 `[154:163]`/`[163:172]`
  - [ ] V1 가격제한폭확대(0043 65B) · M4 장운영스케줄(0019 83B)
  - [ ] **Q2 동적상하한(0042 65B) ← 신규, 이관 아님**
  - [ ] 증권 LP호가 B7(0003 830B)
  - [ ] 채권(0023/0027/0029) → 통계(RID0007/RID0008)
  - [ ] 체결 단독 A3(0036 173B)은 `G7` 을 받으므로 뒤로 미룬다
- [x] **`jeed-krx` 수신부** (2026-09-13, `crates/jeed-krx/src/recv/`) — 소켓 가입, 런타임
      trcode dispatch, spin/block 두 모드, 하트비트를 수신 루프 안에서(§9). 테스트 59 건
      (`tests/recv/`, 전문 빌더는 `tests/decode/common/` 을 `#[path]` 로 재사용).
      **전송계층과 나머지를 갈랐다** — `Pipeline`(필터→디코드→발행→하트비트)에 소켓이 없어서
      jeed-fix 가 같은 자리에 붙고, 링으로 가는 길이 하나로 남는다(§15 전제).
      발행 경로는 `jeed_wire::RecordSink` — `jeed-shm` 이 `RingProducer` 에 구현하므로
      **jeed-krx 는 여전히 jeed-shm 에 의존하지 않는다.** `publish(fill)` 가 `Err` 면 슬롯을
      commit 없이 떨구므로 "부분 갱신 금지" 가 규율이 아니라 타입이 된다.
      필터 순서는 trcode → 길이 → ISIN → 슬롯: 뒤로 갈수록 비싸다.
      `STALE` 판정은 디코더가 아니라 여기서 한다 (§8 — 근거가 아니라 결론을 싣는다)
- [x] **`jeed-fix`** (2026-09-13, `crates/jeed-fix`) — 프로토콜 층 그대로. 베뉴 어댑터 없이
      `MdMessage` 까지. `{error,frame,tagvalue,market_data,session}.rs` + 테스트 50 건
      (`tests/fix/`, CLAUDE.md 의 트리 테스트 규칙대로 `main.rs` 루트).
      **정말로 "그대로" 였다** — 고친 것은 `crate::types::UnixNano` /
      `crate::utilities::converters::Scale` → `jeed_wire` 두 줄과 문서 링크뿐이다.
      베뉴를 모르는 코드라 `AliasMap`·`InstrumentId` 의존이 애초에 없었고, KRX 처럼
      "출력 타입 교체"(§4)를 할 일도 없다. 출구가 `WireRecord` 가 아니라 `MdMessage` 라서.
      `jeed-krx` 와 달리 `VENUE` 상수가 없다 — FIX 는 프로토콜이지 장소가 아니다.
      실캡처 `E:/Data/smbs_fix_db/20260202` 57,221 건(W 42,725 · X 12,457 · HB 2,039)
      offset·len·34·35·272/273 전부 일치, 시퀀스 갭 0, 한쪽만 있는 개장 북 1 건 그대로 살아남음.
      와이어 레코드 대조는 어댑터가 생길 때 같이 온다 (§10 미해결)
- [x] **`jeed-fix` 수신부** (2026-09-13, `crates/jeed-fix/src/recv/`) — TCP 세션, 로그온,
      침묵 감시, 재접속. 테스트 55 건(`tests/recv/`, 루프백 리스너 위에서 실제로 돈다).
      세 가지가 `jeed-krx` 와 다르고, 셋 다 §10 의 "성격이 정반대다" 가 코드로 나온 것이다:
  - **보내야 받는다.** 멀티캐스트는 가입만 하면 오지만 FIX 는 우리 `35=A` 전에는
        한 바이트도 안 온다. 그래서 `recv/emit.rs`(Logon·Heartbeat·TestRequest·
        ResendRequest·SequenceReset·Logout·MarketDataRequest)가 생겼고 KRX 엔 대응물이 없다.
        무할당·시계 안 읽음 — `now` 를 인자로 받고 호출자 버퍼에 쓴다.
  - **대화 논리는 소켓 밖에 둔다.** `Pipeline::ingest` 가 `Reply` 를 **돌려주고** 루프가
        쓴다. 덕분에 세션 프로토콜 전체가 상대 없이 테스트된다. 답이 둘 겹칠 때
        (갭 난 `TestRequest`) 순서는 Logout → ResendRequest → 나머지: 안 답한
        TestRequest 는 다시 오지만 안 물은 재전송은 안 온다.
  - **끊기면 다시 건다.** 멀티캐스트엔 없는 개념이다. `reconnect_ns` 는 지수 백오프가
        **아니다** — 거래소는 정해진 시각에 돌아오는데 그때쯤 백오프가 분 단위면 개장을 놓친다.
      **`unsafe` 가 없고 `#[cfg(windows)]` 도 없다.** `jeed-krx` 가 Winsock 9 개를 직접 선언한
      이유(인터페이스 지정 `IP_ADD_MEMBERSHIP`, `SO_RCVBUF` 되읽기, 다중 소켓 `WSAPoll`)가
      TCP 클라이언트 하나엔 전부 해당 없다. `std::net::TcpStream` 이면 되고, 그래서 테스트가
      루프백 위에서 실제로 돈다.
      **`MdAdapter` 가 베뉴가 붙는 자리다.** KRX 는 전문이 종목과 단위를 말해주지만 FIX 는
      아니다 — `Symbol`→`(venue,isin)`, 없는 `271` 의 의미, 북을 움직이는 `35=X` 를 거부할지가
      전부 베뉴 지식이다. `venue()`/`scales()`/`adapt()` 세 개고, 두 번째 FIX 베뉴가 바꾸는
      유일한 것이다. `STALE` 은 어댑터가 아니라 파이프라인이 씌운다(§8) — 어댑터가 잊을 수
      있는 정책은 정책이 아니라서, 어댑터에 넘기는 싱크가 나이 검사를 통과시키는 래퍼다.
- [x] **`jeed-fix` 리눅스 확인** (2026-09-13) — WSL(cargo 1.97.1)에서 106 건 전부 통과,
      clippy 경고 0. **`jeed-krx`·`jeed-shm` 처럼 `posix.rs`/`windows.rs` 로 가를 게 없었다** —
      `recv/link.rs` 가 처음부터 `std::net::TcpStream` 이라 `unsafe` 도 `#[cfg]` 도 없다.
      실제로 고친 건 **실캡처 경로 하나**뿐이다: 같은 드라이브가 Windows 엔 `E:/Data`,
      WSL 엔 `/mnt/e/Data` 라서 둘을 순서대로 본다. `cfg` 는 필요 없다 — 이 플랫폼이 아닌
      경로는 그냥 존재하지 않고 `is_file` 이 그렇게 답한다. `JEED_SMBS_CAPTURE` 로 덮어쓴다.
      리눅스에서도 `/mnt/e/.../20260202` 57,221 건 그대로 대조 통과.
- [x] **`jeed-fix` 와이어 v2 대응** (2026-09-13) — `Isin`(12B, 공백 패딩) →
      `Symbol`(24B, `NUL` 패딩). 어댑터의 "심볼을 ISIN 열두 바이트에 왼쪽 정렬로 욱여넣는다"
      가 **사라졌다** — 헤더가 이제 베뉴 고유 이름을 그대로 싣는다. FIX 베뉴엔 애초에 ISIN 이
      없었으니 v2 가 없애준 건 우리 쪽 군더더기다. 24B 를 넘는 심볼은 자르지 않고 어댑터가
      거부한다(잘라내면 다른 종목이 된다)
- [x] **`jeed-krx`·`jeed-shm` 리눅스 포팅** (2026-09-13) — WSL(cargo 1.97.1)에서 전부 통과,
      clippy 경고 0. Windows 도 그대로 통과한다(양쪽 다 돌렸다). `unsafe extern` 이 모인 두
      곳만 `windows.rs`/`posix.rs` 로 갈랐다 — `jeed-shm/src/mapping/` 과
      `jeed-krx/src/recv/socket/`. 그 위층(`SharedMapping`·`FeedSocket`·`Poller`·`Receiver`)은
      `#[cfg]` 이 한 줄도 없다.
      **커널이 실제로 다른 지점은 셋뿐이고, 나머지는 이름만 다르다:**
      ① **bind** — 리눅스는 그룹 주소 bind 를 받으므로 목적지가 디먹스에 참여하고
      `IP_MULTICAST_ALL`(`INADDR_ANY` 소켓에 호스트가 가입한 모든 그룹을 꽂아주는 knob)도
      피해 간다. 윈도우는 `INADDR_ANY` 뿐이다(아래 ⚠️). 포트 중복 거부는 **양쪽 다** 건다 —
      둘 중 엄한 쪽을 택해야 conf 하나가 두 플랫폼에서 유효하다.
      ② **객체 수명** — 윈도우 섹션은 마지막 핸들과 함께 죽고, POSIX 이름은 `shm_unlink`
      전까지 `/dev/shm` 에 남는다. 그래서 리눅스에선 `existed()` 가 **더 약한 증거**다
      (죽은 프로세스의 이름이 그대로 남아 있다). 재기동 판별의 권위는 여전히 `boot_id` 다.
      `unlink` 는 윈도우에서 `Ok(())` 무동작 — 호출부를 한 번만 쓰기 위해서다.
      ③ **`SO_RCVBUF`** — 리눅스는 요청값을 **두 배로 기록하고 두 배로 돌려주며**,
      `CAP_NET_ADMIN` 없이는 `net.core.rmem_max`(흔히 208 KiB)에서 잘린다. 8 MiB 를 달라고
      해서 8 MiB 를 받는 게 아니므로 `recv_buffer_bytes()` 되읽기가 여기서 진짜로 일한다.
      그리고 리눅스는 버퍼보다 큰 데이터그램을 **에러 없이 잘라서** 준다(`WSAEMSGSIZE` 상당이
      없다) — 잘린 전문은 길이 검사에 걸려 거부되므로 반쯤 디코드될 일은 없고, 소켓 에러가
      아니라 길이 불일치로 세어진다.
- [x] **`jeed-wire` v2 — 크립토를 받기 위한 ABI 확장** (2026-09-13). 소비자 미부착일 때
      한 번에 몰아서 했다. 다음 번은 공짜가 아니다.
  - 헤더 식별자 `isin: [u8;12]` → `symbol: [u8;24]`. 12바이트로는 크립토 심볼이 안 들어간다
      (`1000000MOGUSDT` 14, OKX 옵션 22). 헤더 48→64B 지만 **레코드는 640B 그대로** —
      꼬리 패딩 48→32 로 흡수. `jeed-krx` 는 `field::{ISIN_LEN, Isin, wire_symbol}` 로
      12바이트 개념을 자기 쪽에 갖는다 (전문 필드 폭은 KRX 상수지 와이어 상수가 아니다)
  - `Venue` 에 `BinanceSpot = 3` / `BinanceFutures = 4`. **스팟·선물을 한 베뉴로 묶으면
      `BTCUSDT` 가 식별자 충돌을 일으켜 소비자가 두 북을 합친다**
  - `WireKind::SnapshotDelta` 예약 해제 → `SnapshotDeltaPayload` 544B 정의.
      16B 델타 레벨 × **32단(양쪽 공유)** + 갱신ID 3개 + 카운트. 공유로 둔 이유는 한 메시지가
      보통 한쪽으로 쏠리기 때문(20:2 는 평범, 16:16 고정이면 거절된다)
  - `jeed-convert` 에 `ParseErr::Precision` + `DynamicExtractor::to_i64_exact`/`to_u64_exact`.
      기존 `to_i64` 는 조용히 자른다 — 고정폭 KRX 필드엔 맞고 크립토엔 틀리다 (CLAUDE.md)
- [x] **`jeed-crypto` — binance** (2026-09-13, `crates/jeed-crypto/`) — spot·USD-M 각각
      bbo / trade / snapshot / delta 8개 디코더. 테스트 74건, clippy·rustdoc 무경고.
      **디코더당 struct 8개가 아니라 `Instrument` 하나 + 자유함수 8개다** — fractal-engine 은
      디코더마다 `instrument_id` + extractor 2개를 복사해 들고 있었는데, 갈리는 건 스트림별
      파싱뿐이라 상태를 한 곳으로 모았다. 스케일은 `exchangeInfo` 에서 오는 **설정**이다
      (KRX 처럼 표준서가 정해주지 않는다).
      가져온 것: `json.rs` 스캐너(첫 바이트 키 디스패치 그대로), 8개 디코더의 필드 해석.
      바꾼 것: 출력 타입(`SnapshotData`/`BboData`/`TradeData` → `WireRecord`),
      `Vec` 레벨 → 고정 배열, `to_i64` → `to_i64_exact`, `#[inline(always)]` → `#[inline]`,
      `s` 심볼 대조 추가(원본엔 없었다 — 배선 사고가 조용히 통과한다).
      **안 가져온 것: `SnapshotCutoff`** (와이어가 10단으로 자르므로 BasisPoint 컷오프는
      소비자 정책), **`recovery/`** (북을 가진 쪽만 리싱크할 수 있다)
- [x] **`jeed-wire` v3 — 베뉴 다섯 개** (2026-09-13). 레이아웃은 안 움직였다.
      `Upbit = 5` · `Bithumb = 6` · `Okx = 7` · `BybitSpot = 8` · `BybitLinear = 9`.
      **모듈 수와 베뉴 수는 다른 질문이다** — OKX 는 `instId` 가 이미 시장을 가르므로
      바이트 하나, 바이비트는 `BTCUSDT` 가 스팟·리니어에서 충돌하므로 전문이 같은데도 둘.
      소비자가 모르는 베뉴 바이트는 이미 거부되는 레코드라(`Venue::from_u8`), 버전을 올리는
      건 "이제 거부하지 말라" 는 신호다
- [x] **`jeed-crypto` — upbit · bithumb · okx · bybit** (2026-09-13). 테스트 67건 추가
      (crypto 141건), clippy·rustdoc 무경고.
  - **`json.rs` 가 두 배로 늘었다.** 첫 바이트 키 디스패치는 바이낸스 전문의 성질이지
      JSON 의 성질이 아니다 — 업비트 `ask_price`/`ask_size`, 바이비트 `topic`/`type`/`ts`,
      OKX `arg`/`action` 이 겹친다. `next_field`(키 전체) · `parse_scalar_bytes`(따옴표
      있든 없든) · `parse_scalar_u64` · `object_at`/`objects_at`(봉투를 부분 슬라이스로) 추가
  - **따옴표 친 정수에 `parse_u64` 를 쓰면 조용히 깨진다.** 닫는 따옴표에서 멈추고 다음
      스캔이 그걸 키의 여는 따옴표로 읽는다. OKX 는 한 객체에서 `ts` 만 따옴표를 친다.
      이걸 `tests/json.rs` 에 함정 째로 박아 뒀다
  - **체결 배치는 반복자로 낸다** (OKX·바이비트). fractal-engine 은 마지막 것만 남겼다 —
      나머지 체결도 시장을 움직였다. `trades(payload)` → `decode_next(inst, recv, out)`
  - **한 구독이 두 kind 를 낸다** (OKX `action`, 바이비트 `type`). 바이비트는 `u == 1` 이면
      `type` 이 뭐든 전체 교체다(서비스 재시작). 모르는 값은 `CryptoError::Unexpected`
  - **fractal-engine OKX 델타의 사슬이 뒤집혀 있었다** — `first_update_id = seqId`,
      `final_update_id = prevSeqId`. 여기선 `first = final = seqId`, `prev = prevSeqId`
      (`pu` 와 같은 슬롯). 둘 다 채워져 있어 아래에서 아무도 불평하지 않았을 것이다
  - **빗썸은 업비트 디코더를 재수출한다.** v2 공개 WS 가 필드까지 같다. 다른 건 베뉴
      바이트뿐이고 그건 `Instrument` 가 정한다
  - **업비트 시퀀스는 지어내지 않는다.** fractal-engine 은 ms 타임스탬프를 `sequence_id`
      자리에 넣었다 — 같은 ms 의 두 북이 같은 북으로 보인다. `quote_ext` 는 `NONE`
  - **안 가져온 것:** OKX `checksum`(북 없는 쪽이 검증 못 하고, 계산 규칙이 OKX 전용이라
      소비자가 `venue` 로 분기해야 읽힌다 — 패딩에 언제든 넣을 수 있다), OKX 레벨의
      주문 건수(`WireDeltaLevel` 에 자리가 없어 스냅샷만 맞고 첫 델타부터 틀려진다),
      바이비트 `seq`(교차 스트림 순서라 자리가 없다), 상태 추적 `last_seq_id`/`prev_seq`
      (갭 탐지는 북을 가진 소비자 몫), `SnapshotCutoff`, `recovery/`, `encode/`
- [x] **`jeed-wire` v4 — 베뉴 네 개** (2026-09-13). `BitgetSpot = 10` · `BitgetLinear = 11` ·
      `GateSpot = 12` · `KucoinSpot = 13` · `KucoinFutures = 14`. 레이아웃은 안 움직였다.
      한때 `delta_flags::UPDATE_ID_VALID` 를 넣었다가 **도로 뺐다** — 크라켄(갱신ID가 아예
      없는 유일한 거래소)을 안 가져오기로 하면서 남은 거래소가 전부 델타에 번호를 매기게 됐고,
      그러면 항상 켜져 있는 플래그가 된다. 항상 켜지는 플래그는 잊어버리는 순간 틀리는 쪽이
      위험해서, 필요해질 때(크라켄) 그 거래소와 같이 들어오는 게 맞다
- [x] **`jeed-crypto` — bitget · gate · kucoin** (2026-09-13). 테스트 89건 추가로 237건
  - **bitget** = OKX 의 전문에 단어만 바뀐 것(`seq`/`pseq`, `price`/`size`, 2원소 레벨).
        `action` 이 없으면 거부한다 — OKX `books5` 와 달리 비트겟은 네 채널 모두 보낸다.
        **fractal-engine 의 OKX 사슬 역전이 여기도 그대로 있었다**(`first = seq`,
        `final = pseq`). 같은 실수 두 번
  - **gate** = 바이낸스의 `U`/`u` 사슬에 `result` 봉투. 체결은 프레임당 하나라 반복자가 없고,
        `create_time_ms` 의 소수점 이하(밀리초 미만)를 **살려서** 싣는다(fractal-engine 은
        점에서 자른다). 북 스냅샷은 REST 뿐이라 `snapshot.rs` 가 REST 본문을 받는다
  - **kucoin** = 현물/선물이 봉투만 같고 안은 전부 다르다. 현물 델타는
        `sequenceStart`…`sequenceEnd` 범위 + 3원소 레벨, 선물 델타는 `"90631.2,sell,2"`
        문자열 하나. 선물 수량은 **계약 수(정수)** 라 스케일 0. 시각은 체결이 나노초,
        선물 북이 밀리초 — 같은 거래소 안에서 갈린다
  - **안 가져온 것:** 체크섬(비트겟 — OKX 와 같은 결론), 시퀀스 갭 추적, `SnapshotCutoff`,
        `recovery/`, `encode/`
- [x] **htx · kraken 은 안 가져온다** (2026-09-13, 결정). htx 는 WS 프레임이 전부 gzip 이라
      압축 해제가 수신부로 들어가야 하고(크레이트에 의존성이 생긴다), kraken 은 레벨이
      객체형(`{"price":…,"qty":…}`)에 시각이 RFC3339, 그리고 **시퀀스가 아예 없어** 델타의
      갱신ID 자리를 비워야 한다 — 셋 다 지금 있는 것과 모양이 다르다. 필요해지면 그때
      `Venue` 다음 바이트로 들어온다
- [x] **`jeed-crypto` 수신부** (2026-09-13, `crates/jeed-crypto/src/recv/`) — TCP · rustls ·
      WebSocket 핸드셰이크 · 프레임 · 조각 조립 · ping/pong/close · 침묵 감시 · 재접속. 테스트
      105 건(`tests/recv/`, 루프백 WS 서버 위에서 실제로 돈다) + 라이브 1 건(`live.rs`, `#[ignore]`,
      바이낸스 `wss://` 에서 체결 3 건 수신으로 TLS 경로 확인). 리눅스(WSL) 동일 통과.
      **결정: `rustls` 만 들이고 프레이밍은 직접 쓴다.** 두 가지가 그 결정을 만든다:
  - **서버 프레임은 마스킹이 없다**(RFC 6455 §5.1). 그래서 페이로드가 수신 버퍼에 디코더가
        원하는 그대로 놓이고, `Frame::payload` 는 그 버퍼의 슬라이스다 — 프레임당 복사도 할당도
        없다. `tungstenite` 는 `read()` 가 `Message::Text(String)` 을 주므로 프레임마다 힙이다.
        마스킹 XOR 은 우리가 보내는 쪽(구독·pong)에만 있고 그건 초당 한 번도 안 된다
  - **TLS 는 손으로 못 쓴다.** `rustls` + `webpki-roots`(모질라 루트, OS 저장소 아님 — Windows 와
        리눅스가 같은 동작). 백엔드는 `aws-lc-rs` 가 아니라 **`ring`**: 전자는 Windows 빌드에
        NASM·CMake 가 필요하고, 어느 기계에서 빌드되느냐를 TLS 백엔드가 정하면 안 된다.
        `logging` 피처도 끈다 — `log` 는 두 번째 외부 크레이트다. `Cargo.lock` 6 → 32 패키지
  - `Sec-WebSocket-Accept` 를 검증한다(§4.1 이 MUST). 그래서 SHA-1 이 60 줄 들어왔다 —
        rustls 의 프로바이더는 SHA-256 이상만 내놓고, 해시 크레이트는 콜드 패스 한 호출에
        의존성 하나다. 표준 벡터로 테스트했다
  - 확장은 안 내민다. 그래서 `permessage-deflate` 가 협상될 수 없고 RSV 비트가 켜진 프레임은
        압축이 아니라 프로토콜 오류다. HTX 를 뺀 이유가 코드로 나온 자리
  - **`Router` 가 베뉴가 붙는 자리다** (`jeed-fix` 의 `MdAdapter` 와 같은 논리). 메시지가 어느
        스트림 것인지는 거래소마다 다르게 말하고(바이낸스 `stream` 봉투, 바이비트 `topic`, OKX
        `arg`, 쿠코인 `topic:심볼`) 한 연결이 여러 스트림을 싣는다. `route(msg, recv_ns, sink)`
        가 `(Instrument, 디코더)` 를 고르고, `keepalive(now, out)` 이 거래소 방언의 ping
        (`{"op":"ping"}`, `ping`, `{"type":"ping"}`)을 낸다. **거래소별 라우터는 아직 없다** —
        바이너리와 같이 온다(아래). 테스트는 바이낸스 현물 체결 하나짜리 가짜 라우터로 돈다
  - **생존은 두 층이다.** RFC 6455 ping 은 모든 서버가 답하므로 `ping_interval_ns` 가 그걸
        보낸다(한 간격 침묵 → ping, 두 간격 → 끊고 재접속. 어떤 프레임이든 시계를 되돌린다 —
        데이터가 오면 살아 있는 것이다). 거래소 방언 ping 은 `Router::keepalive`. 바이낸스·
        업비트는 서버가 먼저 ping 하므로 후자가 필요 없고 기본 구현이 `None` 이다
  - 조각난 메시지는 **유일한 복사**다(`ws::assemble`, 1 MiB). 한 프레임 메시지는 제자리에서
        디코드된다. 조각 사이에 끼는 ping 은 그 자리에서 답한다. 조립 버퍼를 넘치면 잘라서
        디코드하지 않고 끊는다 — 잘린 JSON 은 얕은 북이 아니라 JSON 이 아니다
  - 수신 버퍼 1 MiB 는 **프레임 하나**의 상한이다(바이비트 1000 단 스냅샷 ≈ 50 KB, OKX 400 단
        ≈ 20 KB 의 한 자릿수 위). 압축(consume 마다가 아니라 `fill` 마다)은 한 read 에 프레임
        백 개가 들어왔을 때 꼬리를 백 번 옮기지 않으려는 것 — `jeed-fix` 의 `FrameBuffer` 와
        다른 점
  - `unsafe` 없음, `#[cfg]` 없음 — `jeed-fix` 와 같은 이유. `Link` 는 `Plain(TcpStream)` /
        `Tls(StreamOwned)` 둘 중 하나고 루프는 어느 쪽인지 모른다. 그래서 루프 테스트는 전부
        평문 루프백이고 TLS 는 라이브 한 건이다
  - REST 스냅샷은 여전히 아무도 요청하지 않는다. 이 수신부는 WebSocket 만 안다 — HTTP
        클라이언트를 여기 넣으면 "연결 하나 = 루프 하나" 가 깨진다. 게이트·쿠코인 현물의 시작
        북은 바이너리(또는 라우터)가 가져와서 `snapshot::decode` 에 준다. **§6 `jeed` 바이너리
        항목의 일부다**
- [x] **`jeed` 크레이트 + `jeed-krx` 바이너리** (2026-09-13). conf 로딩, 기동 검증, 세그먼트
      생성, 소켓 가입, 코어 핀, 시그널, 리포트. 테스트 62 건(conf 28 · toml 19 · cpu 6 · log 4 ·
      krx 2 · boot 2 · signal 1), Windows · 리눅스(WSL) 동일 통과. `krx` 테스트는 실제로 링을
      만들고 그룹에 가입하고 루프백으로 `B601F` 를 쏴서 소비자가 링에서 `Quote` 를 읽는다
  - **TOML 리더를 직접 썼다** (`jeed::toml`, ~450 줄). `toml` 크레이트는 `serde` 와 열몇 개
        패키지를 끌고 오고, 우리 파일은 테이블·배열 테이블·인라인 테이블·스칼라·여러 줄 배열이
        전부다. 안 하는 것(여러 줄 문자열, 날짜)은 이름을 대고 거부한다. `conf/krx_trcodes.toml`
        591 코드가 그대로 읽힌다. 외부 의존은 여전히 `rustls` 하나
  - **모르는 키는 기동 실패**, **없는 가드는 켜진 것**(§9 의 사고). `validate` 가 포트 중복(피드
        사이까지 — 윈도우는 두 링에 두 번 발행), 코어 중복, spin 피드 코어 여럿, spin 코어의 SMT
        형제 점유(OS 토폴로지로 판정), 링 크기·이름을 거부한다. 표에 없는 trcode·디코더 없는
        trcode·두 피드에 겹친 trcode 는 경고. 소켓 ↔ trcode 대응은 여전히 못 잡고, 리포트 줄의
        "어느 소켓에서도 안 본 trcode" 가 대신한다
  - 링 생성·소켓 가입은 메인 스레드에서 피드 순서대로 — 실패하면 아무 스레드도 안 뜬다.
        스레드는 스스로 핀하고(`SetThreadAffinityMask` / `sched_setaffinity`), 리포트 줄은
        수신 스레드가 찍는다(spin 은 256 라운드마다 시계). 피드 하나가 죽으면 전부 멈추고
        종료 코드 3. `--check` 는 검증까지만, `--no-pin` 은 개발 기계용
  - `unsafe extern` 은 `cpu/{windows,posix}.rs` `signal/{windows,posix}.rs` 넷뿐이고 그 위층에
        `#[cfg]` 이 없다 — `jeed-shm`·`jeed-krx` 와 같은 갈래
  - 예제 conf 의 `A001F`/`A003F`(파생 종목정보 마스터)는 디코더가 없어 경고가 뜬다 — §10
        "분배그룹번호 매핑" 을 카운터로 확인하려고 가입만 해 둔 것이고, 디코더가 생기면 사라진다
- [ ] **`jeed-fix` 바이너리** — 배선은 있고 입주자가 없다(§10). 어댑터가 붙는 날 `src/bin/fix.rs`
      몇십 줄이다: conf 에 `endpoint`·`sender`/`target`·심볼 필터, 로그온 후 `subscribe`
- [x] **`jeed-crypto` 바이너리 + 거래소별 `Router` + REST 시작 북** (2026-09-13). 워크스페이스
      1,238 건 통과(크립토 라우터·switchboard·HTTP 클라이언트 +100 여 건, `jeed` conf·rules·e2e
      +20 여 건), Windows·리눅스(WSL) 동일, clippy·rustdoc 무경고. `jeed` 의 e2e 는 루프백 WS 서버
      둘과 HTTP 서버 하나를 띄워 바이낸스 체결 · 게이트 REST 시작 북 + 디프 + 체결이 링에 도착하는
      것까지 본다. **실제 거래소 4 곳(바이낸스·게이트·쿠코인·업비트)에 14 초 붙여 봤다** — 전부
      연결·구독·수신, 쿠코인은 티켓 → 소켓 → REST 북까지.
  - **conf 는 네 단어**(`trade` `bbo` `book` `delta`)로 말하고 라우터가 거래소 방언으로 옮긴다.
        거래소가 안 주는 채널·단수는 라우터 생성자가 거부하고 `validate` 가 그 생성자를 부른다
        (`--check` 와 기동이 같은 답). `[[feed.instrument]]` 에 심볼·스케일·채널·단수.
        `venue` 이름 12 개(`binance-spot` … `kucoin-futures`)가 `Venue` 바이트를 고른다
  - **라우터는 요청을 돌려주고 바이너리가 수행한다.** `Router` 트레이트에 `subscriptions()` ·
        `rest_books()`/`route_rest()` · `ticket()`/`endpoint_from_ticket()` 이 기본 구현으로
        붙었다. 소켓·HTTP 는 라우터에 없다 — `jeed-fix` 의 `Reply` 와 같은 논리라 베뉴 대화가
        상대 없이 테스트된다. `recv::VenueRouter` 가 enum 으로 일곱 라우터를 감싼다
        (`route<S>` 가 제네릭이라 `dyn` 이 안 된다)
  - **HTTP 클라이언트를 `recv::http` 에 두었다** — 요청 하나, `Content-Length`·chunked·EOF, 같은
        `rustls`. 외부 의존은 여전히 하나. 수신 루프는 절대 안 부르고 바이너리가 열릴 때마다
        라운드 사이에 부른다. 쿠코인은 연결 전 `bullet-public` 티켓(주소+토큰+ping 주기),
        끊길 때마다 다시 받는다(토큰 만료). 쿠코인 현물 시작 북은 `level2_100`(전체 북 엔드포인트는
        API 키가 필요하고 와이어는 어차피 10단)
  - **배치 규칙을 `conf::rules` 로 뺐다.** KRX conf 와 크립토 conf 가 `Placement` 로 같은
        `check_placement` 를 부른다. `RuleError` 는 하나(KRX 소켓·trcode 변형 + 크립토 종목·라우터
        변형). `feed::{Options, Feed<E>, wait}` 도 두 바이너리가 공유한다
  - **실제 회선에서 찾은 것 둘 (리포트에 "거부 #n: 원인 · 프레임 앞부분" 을 찍게 한 덕에):**
        ① **업비트는 천만 이상 숫자를 지수 표기로 보낸다** — `"ask_price":1.04525E8`. 캡처 프레임
        에는 없던 모양이라 디코더 테스트가 전부 통과하고도 실제로는 북 전부가 거부됐다.
        `json::plain_decimal` 이 자릿수를 옮겨 평범한 십진수로 편 뒤 `Instrument::price/qty` 가
        읽는다(계산이 아니라 이동이라 반올림이 없다). ② 업비트·빗썸은 **binary 프레임**으로 JSON 을
        보낸다 — 루프가 binary 를 버리던 것을 라우터로 넘기게 바꿨다. 그리고 바이낸스 BTCUSDT
        `@depth@100ms` 는 1% 남짓, 게이트는 가끔 **32단을 넘는 델타**를 보낸다 — 설계대로 버려지고
        (`DeltaOverflow`) 소비자가 리싱크할 자리다. 자르지 않는 이유는 CLAUDE.md 에
  - 안 한 것: 바이낸스 `@depth` 의 REST 시작 북(`book` 채널이 있어 안 넣었다; 전체 북이
        필요해지면 `rest_books` 한 줄), 라우터 심볼 조회의 해시 테이블(피드당 종목이 몇 개라 선형
        비교로 충분). ~~쿠코인 선물 실접속~~ → 2026-09-13 저녁 현물·선물 동시 실접속, 둘 다
        티켓 → 소켓 → REST 시작 북 → 발행, 거부 0 (`XBTUSDTM` 1/0 자리)
- [x] **pcap 리플레이 검증** (2026-09-13, `jeed-pcap`, `E:/Data/krx_pcap/20260807.pcap` 92 GB) → **§16.**
      `B604F`/`G704F` 가 5단이 아니라 10단이라는 것(하루의 40% 가 길이 불일치로 전부 버려지고
      있었다)과 채권 디코더가 실회선 전문을 한 건도 못 읽는다는 것을 찾았다. 앞은 고쳤고 뒤는 §16
- [x] **`jeed-fix` 바이너리** (2026-09-13, `crates/jeed/src/{conf/fix.rs,fix.rs,bin/fix.rs}`,
      `conf/fix.example.toml`). 배선은 KRX·크립토와 같고, 어댑터 자리에는 **conf 가 모는
      `SnapshotAdapter`** 를 끼웠다 — 베뉴 바이트·스케일·심볼은 conf 의 사실이고, 35=W → `Quote`,
      35=X 체결 → `Trade` 는 FIX 4.4 의 일반 매핑이며, 북을 움직이는 35=X 와 크기 없는 레벨
      (`default_qty` 미설정)은 **거절**한다(§13 "refusing is sometimes the right answer").
      SMBS 고유 관행은 실접속 날 그 옆에서 특수화한다. 상대 없이 만든 것이라 세션 자체는
      실접속 미검증 — 어댑터·conf 는 `jeed-fix` 의 합성 SMBS 전문으로 테스트했다(17 건)

## 16. pcap 전수 리플레이 (2026-09-13, `jeed-pcap`, `E:/Data/krx_pcap/20260807.pcap`)

§4 의 대조를 드디어 했다. 도구는 `jeed-pcap <capture.pcap> [--trcodes] [--isin] [--dump] [--report]`
(`crates/jeed/src/pcap.rs`): libpcap 리더 + Ethernet/IPv4/UDP 풀기 + **수신 루프와 같은
`Pipeline::ingest`** + trcode 별 집계(길이 분포·종료키워드·결과·거부 사례 8 건씩·포트별 코드).
`pcap` 크레이트 없이 백 줄이고, 92 GB · 1억 9,282만 패킷이 8 분 (Windows, release).

| | |
|---|---|
| 캡처 | 2026-08-06 20:00 UTC → 08-07 07:20 UTC (야간 → 정규). 192,821,066 데이터그램, 81.7 GB |
| 프레임 | not-ipv4 445 (NTP·SNMP), fragment 12, 짧은 것 0, 잘린 것 0. **종료키워드 불일치 0**, 길이 불일치 0 (고친 뒤) |
| 파이프라인 | recv 192.8M · **pub 152.6M** · filtered 17.2M (표에 없는 코드) · unknown 22.7M (표엔 있는데 디코더 없음) · fail 353,539 |

### ① `04F` 는 10단이다 — 하루의 40% 를 버리고 있었다

첫 실행(고치기 전) `B604F` 31,775,900 건 **전부 `wronglen`**, `G704F` 767,138 건 전부. 회선은
554B/661B 를 보내고 디코더는 324B/431B 를 기다렸다. §13 의 "시세는 5단으로 잘라 보낸다" 는
송신채널 시트를 읽은 추정이었고 측정이 아니었다. `depth_for_product_group` · 생성기
(`DOUBLE_LISTED = {"04F": 10}`) · `conf/krx_trcodes.toml` · 테스트 4 건 · CLAUDE.md 를 고쳤고,
두 번째 실행에서 `B604F` 31,775,900 · `G704F` 767,138 전부 발행, pub 120.0M → 152.6M.

디코더 테스트 141 건이 전부 통과한 채로 이랬다. 표준서에서 만든 합성 전문은 표준서를 읽은 대로
만들어지니까 — §4 가 경고한 그대로다. **단수·길이를 옮기면 `jeed-pcap` 을 다시 돌린다.**

### ② 값 대조 (parquet_db 의 fractal-engine 디코더 vs jeed)

`--isin` 18 종목 `--dump` 로 680만 레코드를 TSV 로 뽑아 `(isin, trcode, venue_unix_nano)` 로
조인, 호가 10단 × (가격·수량·건수) 와 체결 (가격·수량·방향·누적) 을 정수 비교. 결과는 아래
"대조 결과" 에 (스크립트 `compare2.py`, 디코더 경로마다 대표 종목 하나, 2만 건 상한).

### ③ ~~채권 디코더는 실회선 전문을 한 건도 못 읽는다~~ → 리더를 고쳤다 (2026-09-14), pcap 재확인은 미완

`B601K` 292,485 · `B601B` 54,212 · `G701B` 2,403 · `G701K` 1,968 · `A301B` 1,811 · `A301K` 660
**전부 `byte 41: invalid digit`**. 헤더 B(41B) 바로 뒤 첫 필드가 실회선에선
`00009987.50` 처럼 소수점 있는 가격인데 디코더는 정수를 기다렸다. 합성 전문 빌더도 같은 오해로
만들어져 테스트는 통과했다 — §4 의 두 번째 사례.

**원인은 표준서에 있었다.** 참고-가격표시정보 시트: 일반채권·소액채권·KTS 가격은
`[부호][정수 7][.][소수 2]` 11B (REPO 만 `[부호][정수 6][.][소수 3]`). `KRX.bond_price` 를
`signed(11, 0)` 에서 `signed(8, 2)` 로 고쳐 `price_scale` 이 `S2` 가 됐고, 빌더도 그 형식으로
다시 썼다. 점 없는 11자리를 넣으면 `byte 41` 에서 튕기는 회귀 테스트를 두었다. 같은 리더가
소액채권(0024/0030) 디코더에도 들어간다(§13 "디코더" 참고). **캡처 드라이브가 안 붙어 있어
`jeed-pcap` 재확인은 못 했다** — 붙으면 `B601K`/`B601M` 이 `fail` 0 인지 본다.

### ④ 야간시장은 `…V` 다 — 표에도 디코더에도 없다

새벽 구간(20:00~ UTC)의 `B603V` 206,505 · `B601V` · `B611V` · `B2xxV` · `R1xxV` … 합계
380,001 건. 길이는 정규와 같고(`B603V` 324B = `B603F`) 포트만 다르다(13301~13377, 정규는
10301~10378). `is_derivative()` 가 `F` 만 보므로 전부 `filtered`. 야간 파생시장(글로벌 거래)이
같은 종목코드로 다른 북을 내는 것이라 세션·보드 ID 를 어떻게 실을지 정한 뒤 붙인다 — 실시간가격제한.md
§5 의 "야간은 정규의 2배" 가 그 자리다. **미결로 §10 에 올린다.**

### ⑤ 소켓 ↔ trcode 대응이 나왔다 — 기동 검증이 못 잡는 그것

리포트의 `ports:` 절이 92 개 멀티캐스트 포트마다 실린 코드를 센다. 파생 A 회선:

| 포트 | 코드 | 건수 |
|---|---|---|
| `233.38.231.92:10304` | `B604F` `G704F` `A304F` `R104F` `V104F` `Q204F` | 33.1M — **하루의 17%** |
| `233.38.231.92:10302` | `B601F` `G701F` `A301F` | 3.1M |
| `233.38.231.116/117:10362/10371` | `B603F` `G703F` `A303F` (옵션은 두 포트) | 4.0M / 5.6M |
| `233.38.231.116/117:10364/10373` | `B605F` `G705F` `V105F` | 7.6M / 7.0M |
| `233.38.231.95:10321` | `N703F` `N712F` `H105F` `B203F` … (통계·예상체결) | 15.5M |
| `233.38.231.152:10402~10404` | 채권 `01K` `01B` `01M` | 0.5M |

`conf/krx.example.toml` 의 주석은 이제 추정이 아니라 이 표다. **"한 번도 안 본 trcode" 경고**의
근거이기도 하다.

### ⑥ 디코더 없는 코드 22.7M — 뭘 안 받고 있나

`N703F` 6.7M · `N712F` 3.9M · `H104F` 2.6M · `N705F` 2.3M · `H105F` 1.2M (파생 통계·예상체결),
`C301Q/S` 1.1M 씩 (증권 정정·취소?), `B204F` 246K (파생 원시호가 10단 651B), `B901Q/S`. 필요해지면
디코더를 붙이고 `jeed-pcap` 으로 확인한다.

### 대조 결과 — 값 불일치 0

| trcode | 종목 | 경로 | 대조 레코드 | 값 | 결과 |
|---|---|---|---|--:|---|
| `B601F` | KR4A01690002 | 파생 5단 `[5].[2]` | 20,000 | 600,000 | 일치 |
| `B604F` | KR4A50680003 | **주식선물 10단, 정수 가격 (고친 것)** | 20,000 | 1,200,000 | 일치 |
| `B603F` | KR4C01689002 | 코스피200 옵션 5단 | 20,000 | 600,000 | 일치 |
| `B605F` | KR4B23680949 | 주식옵션 10단 | 20,000 | 1,200,000 | 일치 |
| `B606F` | KR4A75680004 | 금융선물 5단 | 20,000 | 600,000 | 일치 |
| `B617F` | KR4CAK410032 | 코스닥150 위클리(표에 없던 상품군) | 20,000 | 600,000 | 일치 |
| `B601S` / `B601Q` | 삼성전자 / 코스닥 종목 | 주식 590B 10단 | 20,000 ×2 | 2,400,000 | 일치 |
| `B703S` / `B704S` | ETF / ETN | LP 호가 830B 10단 | 20,000 ×2 | 2,400,000 | 일치 |
| `G701F` / `G704F` / `G706F` | | 체결+호가의 체결 부분 | 20,000 / 20,000 / 16,111 | 224,444 | 일치 |
| `A301S` / `A303S` / `A304F` | | 증권·파생 체결 | 20,000 ×3 | 240,000 | 일치 |

합계 316,111 레코드 · **10,064,444 값 · 불일치 0** (호가 10단 × 가격·수량·건수 양쪽, 체결의
가격·수량·방향·누적). 조인 키 `(isin, trcode, venue_unix_nano)` 가 100% 맞았다는 것 자체가
`HHMMSSuuuuuu` → 절대 ns 조립이 참조와 같다는 뜻이다. `G706F` 의 3,889 건은 jeed 에만 있는데
parquet 빌더가 누적거래량으로 중복 제거한 것(§9①)이라 예상된 차이다. 방향은 참조의 `None` 이
jeed 의 `UNKNOWN(3)` 이다(개장 동시호가 체결).

스크립트: `jeed-pcap … --isin <18 종목> --dump dump.tsv` → `grep` 으로 (trcode, 종목) 별 2만 건
→ pyarrow 로 parquet 을 `venue_unix_nano` 로 조인. 한 번 짜 둔 것이라 세션 스크래치에 있고
저장소엔 안 넣었다 — 다음에 필요하면 `jeed-pcap --dump` 열 순서(`pcap::write_tsv` 문서)로 다시 쓴다.

## 7. feed_handler.md §15 미구현

- [x] **shm 링 구현** (2026-09-13, `crates/jeed-shm`) ← Jeed 의 실제 출구.
      SPSC ×2 는 "세그먼트 2개" 가 아니라 "소비자가 각자 `RingConsumer` 로 붙는다" 로 풀렸다 (§11)
- [ ] **피드 사망 시 기존 포지션 청산을 허용할지** (§9). 시장이 안 보이는 상태의 청산이
      더 위험할 수 있다. 판단은 소비자 쪽이지만 `STALE`·하트비트 신호는 Jeed 가 낸다
- [ ] **KRX 일련번호 전환일 특정** — 우리 회선에서 정보분배일련번호가 채워지기 시작한 날.
      04-01 까지 100% 공백, 05-21 이후 전부 채워짐. 04-02~05-20 캡처 부재로 그 사이로만 좁혀짐 (feed_handler §11②).
      표준서 비고의 "파생시장 시세 송신 port 분리 및 **종목별 보드별 일련번호 제공**" 이 그 변경으로 보인다
- [ ] **DB 전체 유실률 정량화** (05-21 이후 ~40 일) → 백테스트 신뢰구간.
      20260731 표본에서 (종목,보드)별 12.9% / 12.6% / 2.5% 누락, 중복 0, 20 건 연속 패턴 →
      수신 버퍼 오버플로로 보인다. 5 단 전체 스냅샷이라 북이 깨지진 않고 늦는다 (§11③)

## 8. 결정된 것 — G7 을 받는다

**`B6` + `G7`** 로 간다 (`A3` 단독 체결 전문은 안 받는다). 근거는 대역폭이 아니라
**체결과 체결 후 북이 한 메시지에 원자적으로 온다**는 것이다. `A3`+`B6` 로 받으면 체결과
그에 따른 북 갱신이 두 메시지로 갈라져 순서·유실에 노출된다.
`feed_handler.md` §5 가 `WireKind::TradeQuote` 를 따로 둔 이유이기도 하다.

`G7` 단독은 안 된다 — 체결 없이 호가만 바뀐 갱신을 놓치면 북이 늦는다. 그래서 `B6` 는 남는다.

**`Q2`(동적상하한가)도 받는다.** `G7` 이 밴드 값을 인라인으로 실어주지만 **해제 사건은 `Q2` 에만**
있고, 해제되면 지정가·조건부지정가만 가능해진다(→ [krx/실시간가격제한.md](krx/실시간가격제한.md) §5).
값이 아니라 주문 가능 종류가 바뀌는 것이라 `G7` 만으로는 알 수 없다.

### IFMSRPD0037 레이아웃 (431B, 56필드)

```
[0:47]     공통헤더  데이터구분2 정보구분3 일련번호8 보드ID2 세션ID2 ISIN12 종목인덱스6 매매처리시각12
[47:56]    체결가            [56:65]   거래량
[65:74]    근월물체결가      [74:83]   원월물체결가
[83:92]    시가              [92:101]  고가        [101:110] 저가      [110:119] 직전가격
[119:131]  누적거래량        [131:153] 누적거래대금
[153:154]  최종매도매수구분  space 단일가 / 0 해당없음 / 1 매도 / 2 매수
[154:163]  동적상한가        [163:172] 동적하한가
[172:402]  호가부  5단 × (매도가9 매수가9 매도잔량9 매수잔량9 매도건수5 매수건수5 = 46)
[402:411]  매도총잔량  [411:420] 매수총잔량  [420:425] 매도유효건수  [425:430] 매수유효건수
[430]      종료키워드 0xFF
```

> ⚠️ 표준서 인터페이스정의서의 오프셋 컬럼은 **끝 오프셋**이다. 시작 오프셋으로 읽으면
> 한 필드씩 밀린다 (실제로 밀려서 "상한 < 하한" 이 나왔다). 생성기도 이 규칙을 따라야 한다.

레벨 내 필드 순서가 `0034` 와 같다(ask_price, bid_price, ask_qty, bid_qty, ask_cnt, bid_cnt) —
기존 디코더의 레이아웃 가정을 그대로 쓴다.

**`G7` 은 체결이 나야 나온다.** 그래서 호가만 바뀐 갱신을 받으려면 `B6` 가 필요하다.
표준서는 발신 조건을 명시하지 않지만 정황이 셋이고(①`최종`매도매수구분코드라는 이름,
②12M 중복체크 비고가 `0037` 에는 있고 `0034` 에는 없음, ③필드가 `0036` 의 상위집합)
§10 의 측정이 이를 뒷받침한다.

## 9. 측정 (2026-09-13, `E:/Data/krx_parquet_db/20260731` · `E:/Data/krx_pcap/20260731.pcap.zip`)

pcap 은 17 GB zip / 80 GB 원본. `unzip -p | …` 로 스트리밍해서 봤다.
parquet_db 는 **원문 페이로드를 보관하는 데이터셋이 따로 있다**(`krx_price_limit`, `instrument_master`
= `timestamp, trcode, payload`). 호가·체결은 디코드된 컬럼이다.

### ① `A3` 와 `G7` 은 같은 체결 테이프다 — `A3` 는 필요 없다

`trades_01F` 의 `KR4A01690002`(코스피200선물 근월물) 하루치에 대해 `cum[i] − cum[i−1] == qty[i]` 검사:

| 스트림 | 건수 | 불일치 |
|---|---|---|
| `A301F` 단독 | 29,382 | **64.1%** |
| `G701F` 단독 | 93,771 | **20.3%** |
| **합집합** | 123,153 | **0.3%** |

`(isin, venue_ns)` 교집합은 0 인데 합치면 테이프가 이어진다 → KRX 가 나눠 보내는 게 아니라
**parquet 빌더가 누적거래량으로 중복제거**한 결과다(표준서 비고의 "중복데이터 체크"가 이것).
와이어 레벨에서는 `A3` 도 `G7` 도 **모든 체결을 싣는다.** `G7 = A3 + 체결 후 호가 5단` 이므로
`A3` 는 `G7` 의 부분집합이다. 둘 다 받으면 dedup 이 필수이고, 대신 A/B 이중선 같은 유실 복구가 된다.

남은 0.3% 가 진짜 유실이다(§11③ 의 12% 와 같은 성격).

### ② fractal-engine 은 이미 `G7` 을 받고 있다

`quotes_01F`: `B601F` 3,302,685 + `G701F` 93,931 / `trades_01F`: `A301F` 29,384 + `G701F` 93,931.
`G7` 이 호가·체결 **양쪽 테이블에** 같은 건수로 들어간다.

### ③ 동적상하한가 미적용은 `000000.00` 이다 — 공백이 아니다

pcap 3 GB 구간에서 `G7` 전문 16,220 건(종료키워드 검증 통과)을 뜯었다. **공백 0 건.**

```
G701F  KR4A01690002 근월물     상한 000937.00  하한 000928.35   폭 8.65 (≈0.93%)
G711F  KR4A05680009 미니선물   상한 000932.66  하한 000924.04   폭 8.62 (≈0.93%)
G703F  KR4C01686255 옵션       상한 000021.05  하한 000000.01
G701F  KR4A016C0004 원월물     상한 000000.00  하한 000000.00   ← 제도 제외
G711F  KR4D056869S2 스프레드   상한 000000.00  하한 000000.00   ← 제도 제외
```

밴드 폭이 [krx/실시간가격제한.md](krx/실시간가격제한.md) §4 의 "코스피200선물 1%" 와 맞는다.
0 비율도 제도와 맞는다 — `G712F`(미니옵션) 337 건 전부 0, `G716F`(위클리옵션)는 0 건 없음.

실무적으로 **상한 == 0 이면 미적용**으로 판정할 수 있다(살아있는 종목의 상한이 0 일 수 없다).
다만 옵션 하한이 `000000.01` 인 것처럼 0 근처 값이 정상인 경우가 있어 **`trade_flags` 에
`DYN_LIMIT_VALID` 비트 하나를 두는 쪽이 싸고 명확하다.**

### ④ `Q2` 는 parquet_db 에 없다

`krx_price_limit` 는 `V1` 전용이고(`V101F`/`V102F`/`V103F` 원문 payload), 동적상하한 데이터셋은
아예 없다. pcap 에 있는지는 별도 확인 중.

부수 확인: `V103F` 의 정보분배일련번호는 20260731 에도 **공백**이다
(`V103F␣␣␣␣␣␣␣␣G1KR4B01686256…`). `B606F` 는 05-21 이후 채워지는데 `V1` 은 아니다 — §11② 의
"대용량 서비스에서 제공" 단서가 전문마다 다르게 걸린다는 뜻이다.

### ⑤ `B7`(ETF LP 호가)에도 와이어에 없는 필드가 있다

`quotes_03S` = `B703S` 19,270,600 행. 컬럼: 10단 호가 + **`bid_lp_qty`/`ask_lp_qty` 10단** +
`ask_total_qty`/`bid_total_qty` + `expected_price`/`expected_qty` + `mid_price` +
`ask_mid_total_qty`/`bid_mid_total_qty`. LP 수량은 `level_ext::LP_QUANTITY` 로 들어가지만
예상체결가·중간가·총잔량은 자리가 없다 (§10 참조).

## 10. 미결

- ~~**와이어 페이로드 확장 — 동적상하한가는 실어야 한다.**~~ **결정됨 (2026-09-13) → §12**
- ~~**`Q2` 가 우리 회선에 실제로 오는가.**~~ **온다 (2026-09-13 확인) → §14.**
  pcap 전수 스캔에서 1,439 건. §9④ 의 "안 보인다" 는 그 스캔이 상위 40개만 남긴 탓이었다
- **주식파생은 10단(0035/0038).** `B604F`/`B605F`/`G704F`/`G705F` 만 554/661B 로 길이가 다르다.
  `WIRE_MAX_DEPTH = 10` 이라 와이어는 감당하지만 디코더가 갈린다
- **`recv_from`(WouldBlock) 실측 비용.** hot 소켓 개수 상한이 여기서 나온다. 추정만 있고 안 쟀다
- **증권 분배그룹번호 매핑.** ETF 를 받으려면 그 종목이 00006~00010 중 어느 그룹인지 알아야 한다.
  모르면 5 포트를 전부 열어야 한다. 마스터(`A0`)에 실리는지 확인 필요
- **재전송 포트(20301 등) 사용 여부.** feed_handler §11④ 가 "재전송 포트 혼용 시 역전이 생길 수 있다" 고
  적고 있다. 12% 유실을 메우려면 쓰고 싶지만 순서 보장이 깨진다
- ~~**소비자가 Jeed 를 path 의존으로 볼지 git 의존으로 볼지.**~~ → **path 의존 제안 (2026-09-14, §17③).** 두 레포는 모든 머신에 나란히 있고 개발자가 한 사람이다. CI 가 갈라지면 git 의존으로 바꾼다
- ~~**`jeed-fix` 의 출구 — 자리는 났고 입주자가 없다.**~~ → **conf 가 모는 `SnapshotAdapter` 가
  들어갔다 (2026-09-13, §6).** 남은 건 SMBS 실접속에서만 알 수 있는 것 — 35=X 를 어떻게 보내는지,
  크기 없음이 무슨 뜻인지 — 이고, 그때 어댑터를 그 베뉴 옆에서 특수화한다
- **야간 파생시장(`…V` 코드)을 받을지.** 2026-08-07 캡처에 380,001 건, 정규와 같은 길이, 다른
  포트(13xxx). 같은 종목코드로 다른 북이라 세션/보드 ID 를 와이어에 어떻게 실을지가 먼저다 (§16④)
  의 대역이다 — SMBS 를 본떴지만 SMBS 가 아니고, 진짜는 베뉴 옆에 있어야 한다
- **`35=V` 를 누가 보내는가.** 구독은 프로토콜 의무가 아니라 베뉴와의 대화라(깊이·증분 여부·
  엔트리 타입) 루프가 스스로 보내지 않는다. 지금은 `Receiver::subscribe` 를 바이너리가
  로그온 후에 부른다. 베뉴가 늘면 이게 어댑터 훅이 되어야 할 수도 있다
- SMBS 방언 (보류)

## 11. `jeed-shm` 링 설계 (2026-09-13)

구현하면서 내린 결정 두 개. 둘 다 `crates/jeed-shm/src/ring.rs` 모듈 문서에 근거가 있다.

### ① 백프레셔가 아니라 덮어쓰기

`SegmentHeader` 에 write cursor 만 있고 read cursor 가 없다. 처음엔 빠뜨린 줄 알았는데
**그게 맞다.** 백프레셔 링은 꽉 찼을 때 **새 레코드**를 버린다 — 낡은 북을 지키고 방금 시장을
움직인 체결을 버린다는 뜻이다. 시세에선 거꾸로 된 거래다.

그래서:

- 생산자는 **절대 막히지 않고 스스로 버리지도 않는다.** 한 바퀴 돌면 가장 **오래된** 걸 덮는다
- 소비자는 자기가 잃은 걸 레코드의 `producer_seq` 로 안다 — feed_handler.md §7 의 그 코드가
  덮어쓰기 링에서 **정확히** 동작한다. 바뀐 건 아무것도 없다
- `drop_counter` 의 의미는 "링이 꽉 차서 못 쓴 것" 이 아니라 **"링에 넣기 전에 버린 것"**
  (디코드 실패 등). 세그먼트 수명 동안 누적이고 재기동에도 안 지운다.
  `jeed-wire` 쪽 주석을 여기 맞춰 고쳤다

### ② 레코드 자신이 seqlock 이다

덮어쓰기가 사는 대신 생기는 유일한 위험이 **찢긴 읽기**다(소비자가 읽는 중에 생산자가 그 슬롯을
덮는다). `producer_seq` 가 레코드 헤더 오프셋 8 의 자연정렬 `u64` 라 그걸 그대로 seqlock 으로 쓴다:

```
생산자                                  소비자
  seq = u64::MAX (작업 중 센티널)          seq 읽기 → want 와 같아야 한다
  fence(Release)                          레코드 복사 (read_volatile)
  ..본문 쓰기..                            fence(Acquire)
  fence(Release)                          seq 다시 읽기 → 여전히 want 여야 한다
  seq = s
  publish(s + 1)
```

**센티널이 핵심이다.** 없으면 슬롯이 본문은 반쯤 덮인 채 *직전 바퀴의 멀쩡해 보이는 seq* 를
들고 있어서 소비자가 그걸 유효한 레코드로 읽는다. 와이어 포맷은 하나도 안 바뀌었다 — 이미
있던 필드에 두 번째 역할을 준 것뿐이다.

### 부수 결정

- **capacity 는 2의 거듭제곱 강제.** 슬롯 인덱스가 핫 루프의 64비트 나눗셈이 아니라 마스크가
  된다. `jeed_wire::SegmentHeader::slot_offset` 은 일반형(`%`)으로 남겨두고 `jeed-shm` 이
  마스크 버전을 쓴다
- **attach 는 링의 끝(live edge)에서 시작한다.** 붙자마자 한 바퀴치 낡은 북을 재생하면 몇 분 전
  가격으로 판단하게 된다
- **소비자는 read-only 로 매핑한다.** SPSC 규율을 OS 가 강제한다
- **슬롯을 commit 없이 떨구면 아무것도 발행되지 않는다** — seq 는 센티널에 머물고 커서는 안 움직이며
  같은 슬롯이 다음에 다시 나온다. 디코드 실패의 정답 반응이고, CLAUDE.md 의 "부분 갱신 금지" 가
  타입으로 강제된다
- **`boot_id` 가 바뀌면 `Recv::Restarted`.** 소비자가 붙어 있으면 섹션이 살아 있어서 재기동한
  생산자가 같은 메모리에 다시 붙는다(`reused_existing_section()`). 커서만 0 으로 되돌린다
- **의존성은 `jeed-wire` 하나.** `CreateFileMappingW`/`OpenFileMappingW`/`MapViewOfFile`/
  `UnmapViewOfFile`/`CloseHandle`/`VirtualQuery`/`GetLastError` 를 직접 선언했다.
  소비자가 링크하는 크레이트라 `windows-sys` 를 물려주고 싶지 않다
- **`VirtualQuery` 로 실제 매핑 크기를 확인한다.** 세그먼트 헤더는 *다른 프로세스*가 쓴 것이라
  거기 적힌 capacity 를 믿고 인덱싱하면 그게 전부다

### 남은 것

- 64Ki 슬롯 = 36MB/채널. 채널당 실제 필요 깊이는 안 쟀다 (`conf/` 기본값 근거 없음)
- 블로킹 폴백(`WaitOnAddress`/`WakeByAddressSingle`) 미구현. cold 소비자가 busy-spin 하기 싫을 때 필요
- 소비자가 완전히 따라잡았을 때의 `Recv::Empty` 는 **"조용한 시장"과 "죽은 생산자"를 구별하지
  못한다.** 하트비트 레코드가 붙어야 완성된다 (feed_handler §9)

## 12. 와이어 확장 — 동적상하한가 (2026-09-13, 결정)

`TradePayload` 32B → **48B** (`dyn_upper: i64` + `dyn_lower: i64`),
`trade_flags::DYN_LIMIT_VALID`. 따라서 `TradeQuotePayload` 528 → 544B,
레코드 **576 → 640B** (9 → 10 캐시라인, +11%). 48 + 544 = 592 가 64 의 배수가 아니라
**명시 tail 48B** 가 붙는다 (암묵 패딩이면 초기화 안 된 바이트가 와이어에 나간다).

### 왜 실었나

- 동적상하한가는 **주문 가능 범위의 안쪽 울타리**다. 바깥쪽은 `V1` 의 가격제한폭이고,
  주문은 둘 다 통과해야 한다 (§5 세 축, `documents/krx/실시간가격제한.md`).
- **재접속·기동 직후 현재 밴드를 알 길이 `G7` 인라인뿐이다.** `Q2`(IFMSRPD0042)는
  적용·해제 *이벤트*만 보내므로 누적 상태가 없으면 복원이 안 된다. 게다가 `Q2` 가
  우리 회선에 실제로 오는지가 아직 미확인이다 (§9④) — 하나뿐인 경로에 기댈 수 없다.
- **지금이 유일한 기회였다.** 소비자가 붙기 전이라 `WIRE_FORMAT_VERSION` 을 안 올려도 됐다.
  붙은 뒤면 양쪽 바이너리 동시 배포 이벤트가 된다.

### 왜 통계는 안 실었나

`G7` 이 주는 시가·고가·저가·직전가격·근월물/원월물체결가·누적거래대금·호가총잔량(2)·
유효건수(2)는 여전히 자리가 없다. 대부분 **소비자가 체결 테이프로 재구성할 수 있는 값**이라
링 대역만 먹는다. 필요해지면 그때 `WIRE_FORMAT_VERSION` 을 올린다 — 그게 이 상수가 있는 이유다.

### 값 0 과 미적용을 어떻게 가르나

KRX 는 제도 미적용 종목(원월물·선물스프레드)에 공백이 아니라 `000000.00` 을 싣는다 (§9③).
원문만 보면 "밴드 0" 과 "미적용" 이 구별되지 않으므로 **결론을 플래그로 싣는다** —
`DYN_LIMIT_VALID` 가 없으면 `dyn_upper`/`dyn_lower` 를 보지 않는다. §8 의
"근거 말고 결론" 규칙 그대로다.

## 13. `jeed-krx` 디코더 (2026-09-13)

### 5단/10단은 한 디코더다

파생 실시간 전문은 **47B 헤더 + depth × 46B 레벨 블록 + 꼬리** 로 완전히 규칙적이다
(표준서에서 기계로 확인: 헤더 필드명 일치, 레벨 블록 46B, 꼬리 필드명 일치).

| | 5단 | 10단 |
|---|---|---|
| `B6` 우선호가 | `IFMSRPD0034` 324B | `IFMSRPD0035` 554B |
| `G7` 체결+우선호가 | `IFMSRPD0037` 431B | `IFMSRPD0038` 661B |

`324 = 47 + 5×46 + 47`, `554 = 47 + 10×46 + 47`,
`431 = 172 + 5×46 + 29`, `661 = 172 + 10×46 + 29`.

그래서 `DerivativeQuote::new(depth)` / `DerivativeTradeQuote::new(depth)` 하나로 덮는다.
단수는 `depth_for(trcode)` 가 상품군으로 고른다.

### ~~⚠️ 주식선물은 10단 상품인데 시세는 5단으로 온다~~ → **틀렸다. 10단으로 온다 (§16)**

> 아래는 2026-09-13 낮의 판단이고, 같은 날 저녁 `jeed-pcap` 전수 리플레이가 뒤집었다.
> `B604F` 100% 554B, `G704F` 100% 661B. 5단 디코더는 그걸 전부 길이 불일치로 버리고 있었다.
> `depth_for()` · 생성기(`DOUBLE_LISTED`) · 표 · CLAUDE.md 를 10단으로 고쳤다.

**상품마다 거래소가 갖는 호가 단수가 다르다**(지수선물·상품선물 ≠ 주식선물). 하지만 시세 전문은
5단 아니면 10단 둘뿐이고, 깊은 쪽은 실제로 그만큼 쓰지 않는다 — `WIRE_MAX_DEPTH = 10` 으로 충분하다.

주식선물(`04F`)은 두 표준서가 다르게 적고 있다:

| 문서 | `B604F` |
|---|---|
| 정보분배 v1.341 | `IFMSRPD0035` (10단 554B) **한 곳에만** |
| 송신채널 v1.26 | 5단 목록과 10단 목록(`- 주식선물만`)에 **양쪽 다** |

~~실제로 오는 건 **5단**이다.~~ 실제로 오는 건 **10단**이다 — 정보분배 표준서가 맞았다.

```
depth_for(trcode):  04F, 05F, 18F → 10단        나머지 파생 → 5단
```

`04F` 를 틀린 단수로 읽으면 프레임 검사에 걸려 **한 건도 못 받는다**. `B604F` 는 pcap 최다
코드(2,690만 건, 전체의 40%)라 여기가 틀리면 수신량의 절반이 사라진다 — 그리고 그 상태로 디코더
테스트는 전부 통과한다. 표준서에서 만든 합성 전문은 표준서를 읽은 대로 만들어지기 때문이다.

생성기도 같은 규칙을 갖는다(`TRUNCATED_TO_FIVE`) — 안 그러면 `B604F`/`G704F`/`R104F`/`B204F` 가
"길이가 갈리는데 회선으로 못 가름" 으로 `[code_by_channel]` 에 떨어진다.

### 가격 스케일은 표가 아니라 전문에서 읽는다

파생 실시간 가격은 전부 9바이트인데 `[부호][정수8]`(주식선물) / `[부호][정수5].[소수2]`(코스피200) /
`[부호][정수4].[소수3]`(3개월무위험지표금리선물) 세 갈래고, **마지막 건 국채/금리파생과 같은 `06F`** 라
상품군으로도 안 갈린다. 그래서 `.` 위치를 찾는다 — 9바이트 한 번 훑으면 값과 소수자리수가
같이 나오고, 그게 곧 레코드 헤더의 `price_scale` 이다. **종목별 스케일 표가 필요 없다.**

한 전문의 가격은 전부 한 종목의 것이므로 첫 번째로 적힌 가격이 그 전문 전체의 스케일을 정한다.

### 시각 조립 — 날짜도 설정도 없이

매매처리시각은 `HHMMSSuuuuuu`, 날짜가 없다. `clock::absolute_ns(tod, recv_ns)` 가
**수신 시각에 가장 가까운 날**을 고른다. 그러면:

- 자정 랩이 사건이 아니게 된다 (23:59:59.9 를 00:00:00.1 에 받으면 200ms 전으로 앉는다)
- 야간장(18:00~05:00)도 특별 취급이 없다
- **매매일자 설정이 필요 없다** — 롤오버 저녁 18시에 틀릴 설정이 하나 줄어든다

보장은 한 줄이다: 나온 값은 수신 시각에서 12시간 이내다.

### 부분 갱신 금지가 타입으로 강제된다

디코더는 스택에 레코드를 다 만든 뒤에야 `*out = ...` 한다. 어떤 오류든 `out` 은 그대로다.
`jeed-shm` 쪽에서 commit 안 한 슬롯이 발행되지 않는 것과 짝이다 — 디코드 실패의 정답 반응은
**슬롯을 그냥 떨구는 것**이고, 양쪽 다 그렇게 되어 있다.

### 깊이는 세어서 싣는다

책 끝을 넘는 레벨은 공백이 아니라 0 으로 채워 온다. 그래서 `depth` 는 채널 단수가 아니라
**잔량이 있는 레벨 수**(양쪽 중 깊은 쪽)를 센 값이고, 한쪽이 비면 `BID_EMPTY`/`ASK_EMPTY` 가 선다.

### 결정: 예상체결가는 싣는다

`B6` 꼬리의 예상체결가(단일가 결정 이전 지표가격)를 `quote_ext` 에 싣는다
(`quote_ext::EXPECTED_PRICE` 신설). 파생 채널은 `quote_ext` 를 안 쓰므로 **바이트가 0 든다**.
단일가 구간의 정보는 체결 테이프로 복원이 안 되고, 소비자가 안 붙은 지금이 공짜로 늘릴 수 있는
시점이다. 같은 꼬리의 총잔량 ×2 · 유효건수 ×2 는 자리가 없어 여전히 버린다 (§10).

### 아직 없는 것

- **수신부** — 소켓 가입, 런타임 trcode dispatch, spin/block, 하트비트
- **`code_by_channel` 처리** — `B201S` 처럼 회선마다 전문이 다른 코드는 소켓 정보를
  디코더에 같이 넘겨야 한다 (§5 ⚠️). 파생만 받는 동안은 안 걸린다
- **실전 pcap 대조** — 지금 테스트는 표준서에서 만든 합성 전문이다. `E:/Data/krx_pcap` 으로
  길이·종료키워드·필드 통계를 맞춰 봐야 한다

## 14. pcap 전수 스캔 (2026-09-13, `E:/Data/krx_pcap/20260731`, 80GB)

파생 5개 데이터구분 **66,954,832 건**.

| 데이터구분 | 건수 | 비중 |
|---|--:|--:|
| `B6` 우선호가 | 64,807,079 | 96.8% |
| `G7` 체결+우선호가 | 1,304,989 | 1.9% |
| `A3` 체결 | 811,431 | 1.2% |
| `V1` 가격제한폭확대 | 29,894 | 0.04% |
| `Q2` 동적상하한 | 1,439 | 0.002% |

상위: `B604F` 26.9M (주식선물, **10단 554B**) · `B616F` 10.5M (위클리옵션) ·
`B603F` 9.9M (KOSPI200옵션) · `B611F` 7.1M (미니코스피200) · `B601F` 3.3M.

### ① `Q2` 는 온다 — §10 미결 해소

`Q203F` 780 · `Q216F` 558 · `Q204F` 98 · `Q201F`/`Q202F`/`Q211F` 각 1.

§9④ 의 "pcap 에 없다" 는 **그 스캔이 `| head -40` 으로 상위 40개만 남긴 탓**이었다.
소켓을 안 열어서도, 이벤트가 없어서도 아니다. 하루 1,439 건이니 확실히 cold 성격이고,
hot 소켓으로 들어오므로 §5 결론(hot 링에 같이 싣고 소비자가 `kind` 로 분류) 그대로 간다.

§12 에서 동적상하한가를 와이어에 실은 판단은 이 사실로 **약해지지 않는다** — 하루 1,439 건은
"밴드가 거의 안 움직인다" 가 아니라 "적용·해제 *이벤트*가 드물다" 는 뜻이고, 재접속 직후
현재 밴드를 알 길은 여전히 `G7` 인라인뿐이다.

### ② 신규 상품군 `17F`/`18F` — 정보분배 표준서가 송신채널보다 늦다

`B617F` 6,925 · `V117F` 180 · `G717F` 31 이 회선에 오는데 정보분배 v1.341 에 없다.
**송신채널 v1.26 에는 있다** (사용자가 2026-09-13 에 추가):

| 상품군 | | 상장 | 단수 |
|---|---|---|---|
| `17F` | 코스닥150 위클리 옵션 | 2025-10-27 (채널 v1.23) | 5단 |
| `18F` | 개별주식 위클리 옵션 | 2026-06-29 (채널 v1.24) | **10단** |

> ⚠️ **`18F` 는 10단이다.** `depth_for()` 가 `04F`/`05F` 만 10단으로 보고 있어서 틀렸었다.
> 개별주식 위클리옵션은 주식옵션과 같은 개별주식 계열이라 10단으로 온다. 고쳤다 (§13).

그래서 생성기가 **두 표준서를 합친다** — 길이는 정보분배 v1.341, trcode 목록은 양쪽 합집합.
두 문서를 잇는 열쇠는 인터페이스 **이름**이다. v1.26 에서 61개 코드가 보충돼 538 → 591 개.

> ⚠️ **기동 검증을 "표에 없으면 실패" 로 두면 안 된다.** 표준서가 실제 회선보다 뒤처진다.
> 없는 코드는 **경고 + 명시적 허용**으로 간다.

### ③ `B6` 가 전체의 97% 이고 그 중 절반이 `B604F` 하나다

주식선물 호가가 2,690만 건, 그것도 **10단 554B** 짜리다. 링 사이징과 hot 소켓 선택이
여기서 갈린다 — 주식선물을 안 보면 수신량이 절반 이하로 떨어진다.
`conf` 의 `trcodes` 가 "진짜 필터" 인 이유가 숫자로 나온 셈이다 (§5 결과 1).

## 15. 파서 계층 — fractal-engine 에서 가져왔다 (2026-09-13)

`Decimal { value, decimals }` 를 들고 다니며 바이트마다 `.` 를 찾던 방식을 버리고
fractal-engine 의 `utilities::converters` 를 `crates/jeed-convert` 로 포팅했다.
이유·구조는 그 크레이트 문서와 CLAUDE.md 참조.

### ⚠️ 포팅하면서 발견한 것 — fractal-engine 쪽도 확인이 필요하다

두 가지가 나왔고, **둘 다 fractal-engine 의 KRX 경로에 그대로 있다.** jeed 는 고쳐서
가져왔지만 저쪽은 손대지 않았다 (범위 밖).

**① `Config` 의 `integer_size` 는 부호 바이트를 포함해야 한다.**
`is_signed = true` 로 두면 부호 자리가 `total_size` 에는 더해지지만 `clip` 이 읽는 창에는
안 들어간다. 그래서 9바이트 필드를 `build_extractor(true, 5, 2)` 로 만들면 **앞 8바이트만**
읽고, 소수점을 인덱스 5 에서 찾는다 — 실제 `.` 는 인덱스 6 이다.

```
000937.05   (signed, 5, 2) → clip [0..8], '.' 를 5 에서 기대 → InvalidDigit
000937.05   (signed, 6, 2) → clip [0..9], '.' 를 6 에서 기대 → 93705  ✓
```

`EXTRACTOR_CONTAINER.derivative.rate_price` 가 `build_extractor(true, 5, 2)` 이고
`quote.rs` 는 `get_total_size()` 로 잰 9바이트를 넘긴다. 계산대로면 KOSPI200 호가가 전부
`InvalidDigit` 이어야 하는데 운영 중일 테니, **실제로 어떻게 도는지 확인이 필요하다.**

**② 소수점 있는 리더를 점 없는 필드에 대면 값이 조용히 틀린다.**
`squeeze_point` 는 설정된 인덱스의 바이트를 보지도 않고 지운다:

```
000012345  rate(6,2) → 1245   ← '3' 이 지워졌다. 에러 없음
000012345  plain(9,0) → 12345
000937.05  plain(9,0) → InvalidDigit   ← 반대 방향은 알아서 깨진다
```

주식선물·주식옵션·상품파생은 점이 없으므로 이 방향이 실제로 열려 있다. jeed 는
`FixedExtractor::to_i64_checked` 를 추가해 점 자리를 먼저 확인한다.

### 디코더 — 끝났다 (2026-09-13)

```
decode/
  common.rs        헤더 3종 중 47B 공통형, BookAccum
  dispatch.rs      앞 2자리로 1차 분기 → 상품군으로 시장·단수
  derivative/      quote(B6 0034/0035) trade(A3 0036) trade_quote(G7 0037/0038)
                   price_limit(V1 0043) dynamic_limit(Q2 0042)
  securities/      trade(A3 0004) — 주식·ETF 공통 전문 하나
  stock/           quote(B6 0002) trade(→securities)
  etf/             quote(B7 0003) trade(→securities)
  bond/            quote(B6 0023) trade(A3 0027, 세 시장 공통) trade_quote(G7 0029)
                   small_lot/ quote(B6 0024) trade_quote(G7 0030) — 소액채권, 레벨 stride 156
  schedule.rs      M4 0019 — 시장 공통이라 시장 모듈 밖
```

와이어에 자리가 없어 버린 필드 (§10 에 이어서):

- 증권 체결 `[163:185]` 매도/매수최우선호가가격 — **가격만 있고 잔량이 없다.** 잔량 0 으로
  레벨을 만들면 "거기 아무것도 없다"가 되는데 전문은 그런 말을 한 적이 없다. B6/B7 을 같이
  받으므로 버린다.
- 증권 체결 `[148:163]` LP보유수량 — ETN 재고용, 음수 가능
- 증권 우선호가 중간가격·중간가호가총잔량, 채권 호가총잔량
- 채권 체결 거래일자·결제일자·시가/고가/저가 수익률

- 소액채권 우선호가의 **채권종류 블록** (레벨마다 종목 78B 뒤에 같은 78B 가 한 번 더) 과
  채권종류 총잔량 — 전문에 종류 식별자가 없어 `(venue, symbol)` 로 실을 자리가 없다

**아직 안 한 것:** REPO(0025/0031)·금현물·배출권. 시장 하나를 반만 디코드하느니 안 하는 게
낫다 — `dispatch::handles` 가 이들을 claim 하지 않는다. ~~소액채권(0024/0030)~~ 은 2026-09-14 에
붙었다(`bond::small_lot`, 레벨 stride 156 으로 `fill_book` 을 같이 쓴다).

### ⚠️ 윈도우는 멀티캐스트 그룹 주소에 bind 할 수 없다 (2026-09-13 측정)

유닉스 습관대로 `bind(그룹IP:포트)` 를 하면 목적지 주소가 디먹스에 참여해서 소켓이 자기 그룹만
받는다. **윈도우는 이걸 거부한다** — `bind(239.255.77.88:30883)` → `WSAEADDRNOTAVAIL (10049)`.
윈도우가 받는 건 **로컬 인터페이스 주소**다 — `10.20.30.40:포트` 에 bind 한 소켓은 그 NIC 로
가입한 그룹을 받는다 (2026-09-14 `127.0.0.1` 과 LAN 주소로 측정). 그래서 윈도우 bind 는
엔드포인트의 `@인터페이스` 고, 없으면 `INADDR_ANY` 다. 어느 쪽이든 목적지 그룹은 디먹스에
참여하지 않아 **bind 는 포트(와 NIC)로만 거른다.**

테스트는 `@127.0.0.1` 로 가입한다. `INADDR_ANY` 나 LAN 주소에 묶인 UDP 소켓은 윈도우 방화벽이
새로 빌드된 테스트 바이너리마다 "액세스 허용" 프롬프트를 띄우고, 관리되는 기계에선 그걸 답할 수
없다. 루프백은 예외다.

그래서 한 포트에 그룹 둘을 걸면 소켓 둘이 **양쪽 스트림을 각각 다 받아서** 모든 전문이 두 번
디코드되고 두 번 발행된다. 북이 두 배로 보이는 것을 링에서 발견할 일이 아니므로
`Receiver::new` 가 **포트 중복 설정을 거부**한다. 표준 배정은 그룹마다 포트가 다르므로
(10302 선물 / 10322 콜 / 10323 풋) 실무상 걸릴 일은 없다.

`SO_REUSEADDR` 는 남겨 둔다 — 핸들러가 도는 중에 캡처·진단 도구가 같은 포트를 듣게 해 준다.

**리눅스는 그룹 bind 를 받는다** (2026-09-13 WSL 확인). 그래서 `posix.rs` 는 유닉스 습관대로
그룹 주소에 bind 하고, 소켓은 자기 그룹만 받는다. 그런데도 **포트 중복 거부는 양쪽 다 건다** —
둘 중 엄한 쪽이 규칙이어야 conf 파일 하나가 두 플랫폼에서 같은 뜻이 된다. 리눅스에서만 되는
설정을 허용하면 윈도우로 옮기는 순간 조용히 두 배로 발행된다.

### 수신부 설계 시 전제

- **FIX 를 같이 태울 수 있게 짠다.** 소켓 가입·런타임 dispatch·spin/block·하트비트가 UDP
  전용으로 굳으면 jeed-fix 가 들어올 자리가 없다. 전송계층을 추상화하고, 링에 싣는 경로는
  하나로 둔다.
- 하트비트는 수신 루프 **안에서** 찍는다 (별도 스레드는 위장이다).

## 17. 복구 경계와 두 레포의 분할 (2026-09-14, 결정)

fractal-engine 의 `src/data` 피드 부분을 Jeed 에 일임하면서 **복구(recovery)는 fractal-engine 에 남긴다.**
결정권은 두 레포 모두 사용자에게 있다. 경계를 옮길 때는 이 절과 fractal-engine `CLAUDE.md`
"두 레포의 경계" 를 같이 고친다.

### ① 한 줄 규칙

**Jeed 는 `WireRecord` 까지, fractal-engine 은 `WireRecord` 부터.**

| | Jeed (생산자) | fractal-engine (소비자 = OMS) |
|---|---|---|
| 수신 | KRX UDP · FIX 4.4 · 크립토 WS | shm 링 consumer — 라이브 엣지 attach, `Lagged(n)` 처리 |
| 변환 | 원시 바이트 → `WireRecord` | `WireRecord` → `SnapshotData`/`TradeData`/`SnapshotDeltaData` (`wire::adapt::fill_*`) |
| 상태 | **없음.** 북·시퀀스·`last_seq_id` 전부 안 가짐 | 오더북 · 갭 탐지 · 복구 FSM · 전략 · 리스크 · 주문 |
| REST | **시작 북만** — 소켓에 전체 북이 없는 베뉴(Gate)에서 구독 직후 한 번 (`Router::rest_books`) | **갭 복구** — `src/data/recovery/` 의 워커가 `ureq` 로 받아 온다 |
| 의존 | fractal-engine 을 보지 않는다 | `jeed-wire` · `jeed-shm`(consumer) · `jeed-crypto`(`default-features = false`, 디코더만) |
| 안 함 | 북 · 복구 · 주문 · 백테스트 · 정규화 | KRX/FIX/크립토 수신 · 원시 디코드 (이관 후 `src/data/receiver`·`exchanges/*/decode`·`fix` 는 지운다) |

복구가 소비자 몫인 이유는 **빈도가 아니라 구조**다. 갭 판정에는 마지막으로 적용한 갱신 ID 가 필요하고,
스냅샷이 쓸 만한지는 버퍼된 델타와 맞춰 봐야 하고, OKX·Bitget 체크섬은 북이 있어야 계산된다.
무상태 핸들러는 셋 중 어느 것도 할 수 없다. 반대로 "OMS 가 Jeed 에 스냅샷을 요청하는" 역방향 채널은
Jeed 를 요청 처리자로 만들어 무상태를 깨고, 프로세스 경계를 한 번 더 넘어 복구를 늦추며, 결국 언제 요청할지는
여전히 북 쪽이 정한다 — 기각.

### ② 델타 와이어 계약 — 소비자 갭 탐지가 읽는 필드

`SnapshotDeltaPayload` 의 세 ID 와 플래그를 거래소별로 어떻게 채우는지가 계약이다. fractal-engine 의
`GapTracker` 는 이 표를 보고 판정한다. **디코더를 고쳐 이 표가 바뀌면 fractal-engine 이 깨진다** — 표를 먼저 고친다.

| 베뉴 | `first_update_id` | `final_update_id` | `prev_final_update_id` (`PREV_FINAL_VALID`) | 소비자 판정 |
|---|---|---|---|---|
| Binance spot | `U` | `u` | — | Range: `U == last_u + 1` |
| Binance USD-M | `U` | `u` | `pu` ✓ | Chain: `pu == last_u` (fractal 은 현재 Range 로 보고 있다 — `fill_delta` 때 Chain 으로 전환 가능) |
| Gate | `U` | `u` | — | Range |
| KuCoin spot | `sequenceStart` | `sequenceEnd` | — | Range |
| KuCoin futures | `sequence` | `sequence` | — | Monotonic: `seq == last + 1` |
| Bybit | `u` | `u` | — (교차 스트림 `seq` 는 싣지 않음) | Monotonic |
| OKX | `seqId` | `seqId` | `prevSeqId` ✓ | Chain: `prevSeqId == last_seqId` |
| Bitget | `seq` | `seq` | `pseq` ✓ | Chain |
| Upbit / Bithumb | 스냅샷 전용, 델타 없음 | | | 갭 개념 없음 — 다음 스냅샷이 고친다 |
| HTX / Kraken | **받지 않음** (§3) | | | fractal-engine 복구에서도 지웠다 (2026-09-14) |

fractal-engine 쪽 주의: 현재 `GapTracker::Chain` 은 인프로세스 디코더가 `final_update_id` 자리에 prev seq 를
넣는 편법에 기대고 있다. `fill_delta` 어댑터를 만들 때 `prev_final_update_id` 를 읽는 정식 경로로 바꾼다.
`SnapshotDeltaData` 에 그 필드가 없으면 추가한다.

### ③ 제안 — fractal-engine 이 `jeed-wire` 를 path 의존으로 가져간다 → **완료 (2026-09-14)**

fractal-engine `Cargo.toml` 에 `jeed-wire`·`jeed-shm` 이 path 의존으로 들어갔고 복사본은 지웠다
(`src/data/common/wire/` 에는 `adapt`·`convert` 만 남고 나머지는 `pub use jeed_wire::*`). **복사본은 이미
어긋나 있었다** — fractal 쪽 v1/헤더 48B/레코드 576B/ISIN 12B 대 jeed v4/64B/640B/Symbol 24B. 경고로 지키는
ABI 가 며칠 만에 깨진 실증이다. 남은 간극: fractal 의 `AliasMap` 은 아직 12바이트 ISIN 키라 24바이트 심볼은
`AdaptError::SymbolNotIsin` 으로 떨어진다 — 크립토 심볼(`1000000MOGUSDT` 류)을 붙일 때 `Isin` → `Symbol` 로 넓혀야
한다. `quote_ext::EXPECTED_PRICE` 는 `QuoteExtension::KrxExpectedPrice` 로 받는다.

fractal-engine 은 `src/data/common/wire/{header,kind,payload,record,segment,error}.rs` 에 와이어 타입
복사본을 갖고 있다. "두 레포가 같은 레이아웃이어야 한다" 는 경고가 CLAUDE.md 에 박혀 있는데, 경고로 지키는
ABI 는 언젠가 조용히 어긋난다. 제안:

- fractal-engine `Cargo.toml` 에 `jeed-wire = { path = "../jeed/crates/jeed-wire" }`, 링 소비에
  `jeed-shm`, 복구 REST 본문 디코드에 `jeed-crypto = { path = ..., default-features = false }`.
- fractal-engine 의 와이어 복사본은 지우고 **`adapt.rs` 만 남긴다.** `adapt` 는 `InstrumentId`·`AliasMap`
  을 알아야 하므로 소비자 소유가 맞다.
- path 의존을 택하는 이유: 두 레포는 모든 머신(이 PC, Ubuntu 워크스테이션)에 나란히 체크아웃되고 개발자가
  한 사람이다. `WIRE_FORMAT_VERSION` 올림 = 양쪽 동시 배포 이벤트라는 §2 원칙은 그대로다. CI 가 갈라지는 날
  git 의존(태그 고정)으로 바꾼다.
- 백테스트 리플레이(`read_krx_day` → `Vec<WireRecord>`)도 그대로 `jeed-wire` 타입을 만들게 된다. 이미
  바이트 동일성 A/B 를 통과했으니(feed_handler §15) 타입 출처만 바뀐다.

### ④ 복구 REST 본문은 `jeed-crypto` 디코더가 푼다

fractal-engine 의 `SnapshotDecoderSet`(거래소별 REST 스냅샷 파서 10개)은 `jeed-crypto` 디코더 →
`WireRecord(Quote, quote_ext::SEQUENCE = lastUpdateId 류)` → `fill_snapshot` 한 경로로 대체한다.
같은 JSON 을 두 레포가 따로 파싱하지 않는다.

- **이미 REST 본문을 받는 디코더:** Binance spot (`/api/v3/depth`) · Binance USD-M (`/fapi/v1/depth`) ·
  Bitget (`/api/v2/spot/market/orderbook`, `/api/v2/mix/market/merge-depth`) · Gate (`/api/v4/spot/order_book`) ·
  KuCoin spot (`/api/v3/market/orderbook/level2`) · KuCoin futures (`/api/v1/level2/snapshot`).
- [ ] **OKX REST** `/api/v5/market/books` 본문 디코더 — `okx::book` 은 WS `action:"snapshot"` 봉투만 안다.
- [ ] **Bybit REST** `/v5/market/orderbook` 본문 디코더 — `bybit::book` 은 WS `type:"snapshot"` 봉투만 안다.
- [ ] 각 REST 디코더가 `quote_ext::SEQUENCE` 에 채우는 값이 ②표의 델타 ID 와 **같은 카운터**인지 한 줄씩 확인
  (Binance `lastUpdateId`↔`u`, Gate `id`↔`u`, KuCoin `sequence`, Bitget `ts`/`seq`?, OKX `seqId`, Bybit `u`).
  다른 카운터면 소비자가 "스냅샷 이후 델타" 를 가려낼 수 없다.

**복구 스냅샷은 `WIRE_MAX_DEPTH = 10` 단이면 충분하다** (사용자 결정 2026-09-14). REST 가 1,000단을 주더라도
와이어에서 잘리고, 그걸 우회하는 넓은 인프로세스 출력은 만들지 않는다. 10단 너머는 델타가 쌓이며 채워진다.

### ⑤ 링 `Lagged(n)` 은 복구 트리거다

소비자가 늦어 링이 덮어쓰면 거래소는 아무것도 안 떨어뜨렸어도 델타 사슬이 끊긴다. feed_handler §3·§7 대로
**델타 kind 는 리싱크, 스냅샷 kind 는 카운터만.** fractal-engine 에는 시퀀스 비교를 우회하는 외부 트리거
(`OrderBookMessage::TriggerRecovery` → `TradingEngine::trigger_recovery`)가 이미 있다 — 원래 Kraken 체크섬용으로
만든 것인데 Kraken 은 지웠고 이 입구는 `Lagged` 가 쓴다. 되감기 재생은 하지 않는다 (§3 "되감아 재생해도 델타
사슬의 구멍은 안 메워진다").

### ⑥ 와이어는 복구를 모른다

`RECOVERED` 플래그(feed_handler §8)는 **영구히 넣지 않는다.** 복구는 OMS 안에서 시작하고 끝나므로 와이어가
표시할 사건이 없다. FIX resend 가 생기면 그건 핸들러 내부 사건이고, 소비자에겐 `STALE` 해제로만 보인다.

### ⑦ 순서

1. [x] 문서 — 이 절 + fractal-engine CLAUDE.md (2026-09-14)
2. [x] fractal-engine 복구에서 HTX·Kraken 제거 (2026-09-14)
2b. [x] **fractal-engine 인프로세스 수신·디코더 삭제** (2026-09-14, 원래 8단계였으나 앞당김 — 사용자 결정):
    `receiver/`·`decoders.rs`·`fix/`·`flat_file_streamer.rs`·`exchanges/*/decode` WS 디코더·`krx/decode`·HTX·Kraken·`engine/data.rs`,
    214 파일 −33,076 줄. 남긴 것: 주문 인코더, REST 스냅샷 디코더(복구, 6단계까지), KRX FEP·market_state·session, SMBS 식별자.
    `Broker` 는 `WireRecord` 입력 + `DecoderBuffers::fill_from_wire`, 백테스트 `BackTestFeed::Records` 가 같은 어댑터를 탄다.
    fractal-engine 의 SMBS FIX 파서도 지웠으므로 **`jeed-fix` 가 유일한 FIX 구현**이다.
3. [ ] ④ OKX·Bybit REST 디코더 + SEQUENCE 카운터 확인 (Jeed)
4. [x] ③ fractal-engine `jeed-wire` path 의존 전환, 와이어 복사본 삭제 (2026-09-14; 복사본은 이미 v1/v4 로 어긋나 있었다)
5. [ ] `fill_delta` 어댑터 + `GapTracker::Chain` 정식 경로 + ②표 대조 테스트 (fractal-engine)
6. [ ] `SnapshotDecoderSet` → `jeed-crypto` 디코드 경로 교체 (fractal-engine)
7. [x] 링 consumer 연결 + `Lagged` → 복구 트리거 (2026-09-14). **라우터 없이** — fractal-engine 의 `Broker` 스레드와
   `take_feed_producers` 를 지우고 각 `TradingEngine` 이 `FeedInput`(`engine/feed.rs`)으로 세그먼트마다 자기
   `RingConsumer` 를 붙인다(feed_handler §2 그대로, 사용자 결정 "TE 가 직접 가져가는 게 효율적", TE 1~3개).
   `Lagged(n)`·`Restarted` 는 델타 FSM 이 있는 종목 전부 `trigger_recovery`; 스냅샷 kind 는 카운터만.
8. ~~[ ] 인프로세스 수신·디코더 삭제~~ → 2b 로 앞당겨 완료.

### ⑧ 주문 와이어는 fractal 소유 — jeed-shm 은 매핑만 빌려준다 (2026-09-14)

OMS ↔ 주문 게이트웨이 전송(`crates/order-wire` ABI, `crates/order-shm` 백프레셔 SPSC 큐)은 **fractal-engine 소유**다.
jeed 에는 주문을 아는 코드가 없다. 시세 링(덮어쓰기·seqlock)은 주문에 맞지 않아 큐는 따로 만들되, 플랫폼 코드
(Win32 섹션 · `shm_open` · 크기 검증 · `SegmentName`)를 두 벌 두지 않으려고 `jeed_shm::SharedMapping` 을 빌린다.
그래서 jeed-shm 에 추가한 것은 **`SharedMapping::open_rw(name)` 하나** — `open` 과 같되 읽기·쓰기 매핑(큐의
소비자는 자기 읽기 커서를 세그먼트 안에 쓴다). 링 자체는 이 함수를 쓰지 않는다. 상세는 fractal-engine
`documents/gateway/todo.md` §1.
