# Jeed

**J**unbeom f**eed** handler — KRX UDP 멀티캐스트 · FIX 4.4 · 크립토 거래소 WebSocket 을
받아 **고정 레이아웃 640 바이트 와이어 레코드**로 만들어 공유 메모리 링으로 넘기는 피드
핸들러. 소비자(OMS)와 **별도 프로세스**로 돈다. 순수 Rust, 외부 의존은 `rustls` 하나.

```text
 KRX 회선 ──UDP──▶ jeed-krx    ─┐
 크립토 거래소 ──WSS──▶ jeed-crypto ─┼─▶ 디코드 · 필터 · 정규화 ─▶ WireRecord ─▶ shm SPSC 링 ─▶ 소비자
 FIX 베뉴 ──TCP──▶ (jeed-fix)  ─┘        무상태 · 북 없음                      프로세스 경계
```

## 왜 프로세스를 나누나

- **파싱 버그 하나에 OMS 를 재배포하지 않는다.** 소비자는 `jeed-wire` + `jeed-shm` 만 의존한다.
  KRX 전문 레이아웃, 거래소 JSON 방언, TLS — 전부 링 이쪽에 갇힌다.
- **핫 패스가 한 코어에 핀된 busy-spin 하나다.** 링 생산자는 막히지 않고, 소비자는
  `producer_seq` 로 자기 유실을 안다.
- **와이어 포맷이 락인이지 링이 락인이 아니다.** `WIRE_FORMAT_VERSION` 을 올리는 건 양쪽
  바이너리 동시 배포 이벤트라서 가볍게 올리지 않는다.

설계 근거 전체는 [`documents/feed_handler.md`](documents/feed_handler.md), 작업 이력과
측정은 [`documents/todo.md`](documents/todo.md), 코드를 만질 때 알아야 하는 함정은
[`CLAUDE.md`](CLAUDE.md) 에 있다.

## 크레이트

```text
crates/
  jeed-wire/     와이어 레코드 ABI. 의존성 0. 소비자가 의존하는 유일한 것
  jeed-convert/  고정폭 ASCII 수치 파서 (SWAR)
  jeed-shm/      named mapping(Win32) / shm_open(POSIX) + SPSC 링 (producer / consumer)
  jeed-krx/      KRX UDP 수신 + 전문 디코더 → WireRecord
  jeed-fix/      FIX 4.4 프로토콜 → MdMessage (베뉴 어댑터 자리는 있고 입주자가 없다)
  jeed-crypto/   크립토 WebSocket JSON → WireRecord
                 binance · upbit · bithumb · okx · bybit · bitget · gate · kucoin
                 + recv/ WS·TLS 수신부, 거래소별 Router, 한 번 쓰는 HTTP 클라이언트
  jeed/          바이너리. conf(TOML 리더 직접 씀) · 코어 핀 · 세그먼트 · 시그널 · 리포트 배선
                 src/bin/krx.rs → jeed-krx,  src/bin/crypto.rs → jeed-crypto
```

```text
jeed-wire ←── jeed-shm ←──┬── jeed (bin)
    ↑                     │
    ├── jeed-krx ─────────┤
    ├── jeed-fix ─────────┤
    └── jeed-crypto ──────┘
    ↑
    └──────────────────── 소비자는 jeed-wire + jeed-shm 만
```

핸들러 크레이트(`jeed-krx` `jeed-fix` `jeed-crypto`)는 **`jeed-shm` 에 의존하지 않는다.**
`jeed_wire::RecordSink` 에 쓰고, 바이너리가 그 싱크를 링에 연결한다. 그래서 디코더는 전부
캡처된 바이트 위에서 소켓 없이 테스트된다.

## 와이어 레코드

`WireRecord` 는 640 바이트, `#[repr(C)]`, 캐시라인 정렬, 암묵 패딩 0 (컴파일 타임 단언).

```text
RecordHeader 64B   kind · venue · symbol[24] · recv_ns · venue_ns · producer_seq · price/qty scale · depth · flags
payload     544B   Quote(10단 양쪽) · Trade · TradeQuote(체결+북 원자) · SnapshotDelta(32단 디프) · Heartbeat · …
tail         32B   명시 패딩
```

- **결론만 싣고 근거는 안 싣는다.** `STALE` 플래그는 싣고, 거래소별 시퀀스 갭 판정은 안
  싣는다 — 소비자가 `venue` 로 분기해야 읽히는 비트는 공통 필드가 아니다.
- **식별자는 `(venue, symbol)` 원본.** 프로세스 로컬 인터닝 id 는 경계를 못 넘는다.
  바이낸스 현물과 USD-M 은 `BTCUSDT` 가 겹치므로 베뉴 바이트가 다르다.
- **스케일은 값이 아니라 헤더의 속성.** 가격은 정수로 가고, 자릿수는 헤더에 한 번 찍힌다.
- **델타는 자르지 않고 버린다.** 스냅샷은 얕아져도 북이지만, 잘린 델타는 영영 틀린 북이다.

## shm 링

- **백프레셔가 아니라 덮어쓰기.** 꽉 차면 가장 오래된 걸 덮는다. 생산자는 절대 막히지 않는다.
- **레코드의 `producer_seq` 가 슬롯 seqlock.** 쓰는 동안 `u64::MAX` 센티널, 본문, Release,
  진짜 seq. 소비자는 앞뒤로 두 번 읽어 찢긴 읽기를 걸러낸다.
- **소비자는 링의 끝(live edge)에서 붙는다.** 낡은 한 바퀴를 재생하지 않는다.
- **`boot_id`** 로 생산자 재기동을 안다. 하트비트 레코드로 "조용한 시장" 과 "죽은 생산자" 를 가른다.

```rust
use jeed_shm::{Recv, RingConsumer, SegmentName};
use jeed_wire::WireRecord;

let name = SegmentName::local("jeed.krx.hot")?;
let mut rx = RingConsumer::attach(&name)?;
let mut rec = WireRecord::zeroed();
loop {
    match rx.try_recv(&mut rec) {
        Recv::Record => { /* rec.kind(), rec.header.venue(), rec.quote() … */ }
        Recv::Lagged(n) => { /* n 개를 놓쳤다 — 델타 사슬이면 리싱크 */ }
        Recv::Restarted { .. } => { /* 생산자가 재기동했다 */ }
        Recv::Empty => { /* spin 또는 yield */ }
    }
}
```

## 바이너리

| 바이너리 | conf | 수신 | 한 피드 = |
|---|---|---|---|
| `jeed-krx` | [`conf/krx.example.toml`](conf/krx.example.toml) | UDP 멀티캐스트 가입, 런타임 trcode 디스패치 | 소켓 묶음 하나 · 스레드 하나 · 링 하나 |
| `jeed-crypto` | [`conf/crypto.example.toml`](conf/crypto.example.toml) | WebSocket 하나, 거래소별 `Router` | 연결 하나 · 스레드 하나 · 링 하나 |

```bash
cargo build --release
./target/release/jeed-krx    conf/krx.toml    --check    # 검증만, 아무것도 안 만든다
./target/release/jeed-crypto conf/crypto.toml --no-pin   # 코어 핀 없이 (개발 기계)
```

종료 코드: `0` 요청으로 정지 · `1` 사용법/conf · `2` 기동 실패 · `3` 피드 사망.
피드 하나가 죽으면 전부 멈춘다 — 소비자가 반쪽 시장을 보게 두지 않는다.

**conf 규칙은 둘 다 같다.** 모르는 키는 기동 실패(`ring_slot` 이 기본값 옆에서 조용히
무시되면 안 된다), 없는 `[health]` 가드는 **켜진** 값(100 ms 하트비트 / 500 ms stale),
끄려면 `0` 을 적는다. 코어 중복, spin 피드에 코어 여럿, spin 코어의 SMT 형제 점유, 2의
거듭제곱이 아닌 링은 `--check` 가 거부한다.

**할 수 있는 건 전부 스레드 전에 실패한다.** 링 생성·소켓 가입·라우터 구성은 메인
스레드에서 conf 순서대로 한다. 두 번째 피드가 틀리면 아무것도 도는 것 없이 종료한다.

### `jeed-krx`

한 포트에 데이터구분이 전부 섞여 오므로 `sockets`(가입할 곳)와 `trcodes`(건질 것)가
따로다. 소켓 ↔ trcode 대응은 회선 배정이라 검증할 수 없고, 리포트 줄이 "어느 소켓에서도
한 번도 안 본 trcode" 를 찍어 오배선을 드러낸다. `mode = "spin"` 은 코어 100% 의 busy-spin,
`"block"` 은 `WSAPoll`/`poll`.

### `jeed-crypto`

conf 는 네 단어의 채널 어휘로 말하고 라우터가 거래소 방언으로 옮긴다:

| venue | `trade` | `bbo` | `book` | `delta` |
|---|---|---|---|---|
| binance-spot / -futures | `@trade` / `@aggTrade` | `@bookTicker` | `@depth{N}@100ms` | `@depth@100ms` |
| upbit / bithumb | `trade` | – | `orderbook` | – |
| okx | `trades` | – | `books5` | `books` |
| bybit-spot / -linear | `publicTrade` | – | `orderbook.{N}` | – |
| bitget-spot / -linear | `trade` | – | `books` / `books{N}` | – |
| gate-spot | `spot.trades` | – | – (REST) | `spot.order_book_update` |
| kucoin-spot / -futures | `/market/match` · `/contractMarket/execution` | – | – (REST) | `/market/level2` · `/contractMarket/level2` |

거래소가 그 스트림을 안 주면 `--check` 에서 거부된다. `delta` 는 디프만 오는 채널이라
시작 북이 따로 필요한데, 게이트·쿠코인은 연결될 때마다 REST 로 한 번 받아 같은 링에
`Quote` 로 싣는다. 쿠코인은 소켓 주소 자체를 REST 티켓(`bullet-public`)으로 받는다.
구독 메시지·방언 ping(`ping`, `{"op":"ping"}`, `spot.ping`, `{"type":"ping"}`)·REST 요청은
전부 라우터가 *무엇을* 보낼지 말하고 바이너리가 라운드 사이에 보낸다 — 수신 루프 안에
블로킹 호출이 없다.

## 빌드 · 테스트

```bash
cargo build
cargo test --workspace          # 1,200+ 건. 소켓 테스트는 루프백, 멀티캐스트는 239.255/16
cargo clippy --workspace --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Windows 와 리눅스(WSL)에서 같은 결과여야 한다. OS API 는 `jeed-shm/src/mapping/`,
`jeed-krx/src/recv/socket/`, `jeed/src/{cpu,signal}/` 의 `windows.rs` / `posix.rs` 에만 있고
그 위층에 `#[cfg]` 이 없다. 실제 거래소에 붙는 테스트는 `#[ignore]`
(`cargo test -p jeed-crypto --test recv live -- --ignored`).

`tests/` 는 `src/` 구조를 그대로 따른다. 전문 빌더와 캡처 프레임은 `tests/<venue>/common/`
에 한 벌 두고 다른 테스트 크레이트가 `#[path]` 로 가져다 쓴다 — 복사본이 생기면 같이
고쳐야 하는데 안 고쳐진다.

## 저장소 안내

```text
documents/
  feed_handler.md      설계 근거 (§ 번호는 todo.md 가 가리킨다)
  todo.md              작업 이력 · 결정 · 측정 (pcap 전수 스캔, 유실률 …)
  krx/                 KRX 표준서 발췌, 레이아웃 표, 제도 문서
conf/
  krx_trcodes.toml     표준서에서 생성 (tools/gen_krx_trcodes.py). 손으로 안 쓴다
  krx.example.toml     jeed-krx 템플릿. IP·포트는 회선 배정표를 보고 사람이 적는다
  crypto.example.toml  jeed-crypto 템플릿
tools/                 표준서 xlsx → TOML 생성기, 레이아웃 덤프
```

## 아직 없는 것

- `jeed-fix` 바이너리 — 배선은 있고 베뉴 어댑터(`MdAdapter`)가 없다. SMBS 는 실물 확인 후.
- pcap 리플레이 대조 (`E:/Data/krx_pcap`), 소액채권·REPO·금현물 디코더, HTX·Kraken.
- 링의 블로킹 폴백(`WaitOnAddress`), 피드 사망 시 청산 정책 — 소비자 쪽 결정.

## License

MIT
