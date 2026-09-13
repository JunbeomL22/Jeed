# CLAUDE.md

This file provides guidance to Claude Code when working with this repository.

## Project Overview

Jeed (**J**unbeom f**eed** handler) — KRX UDP 멀티캐스트와 FIX 4.4 를 받아 **고정 레이아웃
와이어 레코드**로 만들어 공유 메모리로 넘기는 피드 핸들러. 소비자(fractal-engine OMS)와
**별도 프로세스**로 돈다. HFT 경로이므로 성능이 중요하다.

**범위 밖:** 주문(KRX FEP/HOA), 오더북, 정규화(`SnapshotData` 변환), fractal-engine 수정.
피드 핸들러는 무상태다 — 디코드 + 필터 + 정규화만 하고 북을 갖지 않는다.

설계 근거는 [`documents/feed_handler.md`](documents/feed_handler.md), 작업 목록은
[`documents/todo.md`](documents/todo.md).

## Project Structure

```
crates/
  jeed-wire/   와이어 레코드 ABI. 의존성 0. 소비자가 의존하는 유일한 것
  jeed-shm/    Windows named mapping + SPSC 링 (producer/consumer)
  jeed-krx/    KRX UDP 수신 + 전문 디코더 → WireRecord
  jeed-fix/    FIX 4.4 프로토콜 → MdMessage → WireRecord
  jeed/        바이너리 (krx, fix)
documents/     설계·표준서·제도 문서
conf/          채널 테이블(생성) + 배포 설정(수기)
```

`tests/` 구조는 `src/` 구조를 그대로 따라간다. `src/` 에 모듈을 추가하면 같은 상대 경로에
테스트 파일을 만든다.

## Build Commands

```bash
cargo build
cargo test
cargo check
```

## KRX 전문을 다룰 때 반드시 아는 것

### 정보분배일련번호는 **공백일 수 있다**

`[5:13]` 8바이트. 스페이스로 채워져 오는 경우가 있다. 표준서가 "시세 : 종목별 보드별 부여
(※ **대용량 서비스에서 제공**)" 이라고 단서를 달고 있고, 그 단서가 **전문마다 다르게 걸린다.**

측정된 사실:

| 전문 | 상태 |
|---|---|
| `B606F` (파생 우선호가) | 2026-04-01 까지 100% 공백 → 05-21 이후 채워짐 |
| `V103F` (가격제한폭 확대) | **20260731 에도 공백** |
| `V101F` / `V102F` | 채워짐 |

**공백을 0 으로 파싱하면 매 메시지가 갭으로 보인다.** 공백은 "갭 없음" 이 아니라
**"측정 불가"** 다. `Option`/센티널로 구분해서 다뤄야 하고, 순서의 권위로 쓰면 안 된다
(순서는 `producer_seq`, 시각은 `recv_ns`). 애초에 이 번호는 (종목 × 보드) 단위라
채널 liveness 신호로도 못 쓴다 — 종목이 조용하면 번호도 멈춘다.

### 표준서 인터페이스정의서의 오프셋 컬럼은 **끝 오프셋**이다

시작 오프셋으로 읽으면 필드가 한 칸씩 밀린다. 조용히 틀리므로 (밀린 채로 파싱돼서 값이
나오긴 한다) 전문 레이아웃을 옮길 때마다 확인한다.

### 전문 끝은 `0xFF`, 길이는 전문마다 고정

`KrxDataParse::validate` 패턴 — **길이 + 종료키워드를 먼저 보고** 그 다음에 필드를 읽는다.
FIX 의 `BodyLength` + `CheckSum` 과 같은 자리다.

### 값이 없을 때 공백이 아니라 `000000.00` 이 오기도 한다

동적상하한가(`G7` `[154:163]`/`[163:172]`)는 제도 미적용 종목(원월물·선물스프레드 등)에
공백이 아니라 `000000.00` 을 싣는다. "값 0" 과 "미적용" 이 구별되지 않으므로 유효 플래그로
결론을 실어 보낸다.

### 파싱 실패 시 출력 버퍼를 부분 갱신하지 않는다

절반만 채워진 레코드가 링에 나가면 소비자가 그걸 유효한 시세로 읽는다.

## Coding Guidelines

### 와이어 레코드

- `#[repr(C)]` · 고정 크기 · 명시 패딩. **암묵 패딩 0** 을 컴파일 타임에 단언한다
  (필드 폭 합 == `size_of`). 그래야 `as_bytes()` 가 건전하다.
- `WIRE_FORMAT_VERSION` 을 올리는 것은 **양쪽 바이너리 동시 배포 이벤트**다. 가볍게 올리지 않는다.
- 와이어에는 **결론만 싣고 근거는 안 싣는다.** 소비자가 `venue` 로 분기해야 읽히는 비트는
  공통 필드가 아니다 (`STALE` 은 싣고, `VENUE_SEQ_GAP` 은 안 싣는다).
- 프로세스 로컬 식별자(인터닝 id)는 경계를 못 넘는다. 원본 `(venue, isin)` 을 싣는다.

### Performance

- 핫 패스에서 힙 할당을 피한다. `Vec`/`Box`/`String` 대신 스택 배열·참조.
- 작은 핫 함수에 `#[inline]`. **`#[inline(always)]` 는 쓰지 않는다.**
- 16 바이트 이하 작은 타입은 `Copy`.
- 수신 루프는 코어에 핀된 busy-spin 이다. 루프 안에 블로킹 호출을 넣지 않는다.

### Time

- `UnixNano` 는 `u64` 라 `a - b` 가 랩한다. **시각 차는 전부 `saturating_sub`.**
- 시스템 시계는 **단조가 아니다.** 순서의 권위는 `producer_seq`, `recv_ns` 는 측정·라벨링용.
- KRX 원문 `매매처리시각` 은 `HHMMSSuuuuuu` 로 날짜가 없어 자정에 랩한다. 절대 ns 조립은
  피드 핸들러 안에서 끝낸다.
