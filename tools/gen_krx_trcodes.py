#!/usr/bin/env python3
"""KRX 표준서 → conf/krx_trcodes.toml 생성기.

    python tools/gen_krx_trcodes.py

**두 표준서를 합친다.** 둘은 갱신 주기가 다르고, 새 상품은 송신채널 쪽에 먼저 뜬다:

| 원본 | 주는 것 | 판 |
|---|---|---|
| 접속표준서(정보분배-UDP_TCP실시간) 인터페이스목록 | 인터페이스 → 이름·**길이**·주기 | v1.341 |
| 접속표준서(UDP) 공통정보(송신채널_정보구분) 전용선 시트 | 인터페이스 **이름 → trcode** | v1.26 |
| 〃 정보구분코드 시트 | 상품군 3바이트 → 이름 | v1.26 |

정보분배 v1.341 에는 코스닥150 위클리옵션(`17F`)도 개별주식 위클리옵션(`18F`)도 없는데
회선에는 온다. 송신채널 v1.26 에는 둘 다 있다. 그래서 길이는 v1.341 에서, 코드 목록은
양쪽 합집합으로 만든다. 두 문서를 잇는 열쇠는 **인터페이스 이름**이다.

**멀티캐스트 IP·포트는 생성하지 않는다.** 송신채널 문서에 들어 있지만 읽지 않는다 —
회선 배정이라 표준서 값이 우리 회선에 그대로 오지 않고, 생성기가 만들어내면 틀린 값이
조용히 박힌다. 배포 설정(`conf/krx.toml`)은 사람이 적는다.

Rust 가 아니라 파이썬인 이유: 표준서 버전이 올라갈 때만 한 번 도는 스크립트다.
`calamine` 을 워크스페이스 의존성으로 들이는 값을 못 한다. 결과 TOML 이 커밋되고,
그게 빌드가 보는 유일한 입력이다.

trcode 5바이트 = [데이터구분 2][정보구분+시장구분 3].
  B601F → B6(우선호가) + 01F(kospi200선물)
"""

from __future__ import annotations

import datetime as _dt
import re
import sys
from pathlib import Path

try:
    import openpyxl
except ImportError:  # pragma: no cover
    sys.exit("openpyxl 이 필요하다: pip install openpyxl")

ROOT = Path(__file__).resolve().parent.parent
SPEC = ROOT / "documents/krx/접속표준서(정보분배-UDP_TCP실시간)_v1.341-배포용.xlsx"
SPEC_VERSION = "1.341"
CHANNELS = ROOT / "documents/krx/접속표준서(UDP) 공통정보(송신채널_정보구분)_v1.26.xlsx"
CHANNELS_VERSION = "1.26"
OUT = ROOT / "conf/krx_trcodes.toml"

# 인터페이스목록 시트 컬럼 (0-based). 헤더가 두 줄 병합이라 인덱스로 잡는다.
COL_ID, COL_NAME, COL_PERIOD, COL_TRCODE, COL_MARKET, COL_LENGTH = 1, 2, 6, 8, 10, 30
FIRST_DATA_ROW = 5

# 전용선 시트 컬럼. **3(그룹IP)·4(운용포트)는 일부러 읽지 않는다.**
CH_NAMES, CH_CODES = 8, 9

CODE_RE = re.compile(r"^[A-Z0-9]{5}$")
# "(파생A) DRV : G701F, G702F"  /  "          KNX : B001X"  /  "LK000"
LINE_RE = re.compile(r"^(?:\((?P<grp>[^)]+)\)\s*)?(?:(?P<mkt>[A-Z]{2,8})\s*:)?\s*(?P<codes>.*)$")

# 파생 호가 전문의 단수는 상품군이 정한다. 근거는 송신채널 v1.26 전용선 시트가
# "파생 우선호가 (우선호가 10단계)" 에 붙여 놓은 코드 목록이고, 이 스크립트가
# 아래 기대값과 대조해서 달라지면 경고한다 — 디코더의 `depth_for()` 가 이 집합이다.
# `04F`(주식선물)는 빠져 있다 — 상품은 10단이지만 시세는 5단으로 잘라 보낸다(CLAUDE.md).
# 송신채널 v1.26 은 `B604F` 를 5단·10단 목록에 **둘 다** 올려 두므로 여기서 빼 준다.
TEN_DEEP_EXPECTED = {"05F", "18F"}

# 원장은 10단이지만 **시세는 5단으로 잘라서** 오는 상품군. 송신채널 v1.26 이 `B604F` 를
# 5단·10단 목록에 둘 다 올려 두는 탓에 표만 보면 못 가른다 (CLAUDE.md).
TRUNCATED_TO_FIVE = {"04F"}

# jeed-krx 가 디코드하는 두 계열. `[derivative_depth]` 는 이것만 보고 만든다 —
# 디코더의 `depth_for()` 가 덮는 범위와 같아야 하기 때문이다.
DEPTH_INTERFACE_NAMES = {
    "파생 우선호가 (우선호가 5단계)": 5,
    "파생 우선호가 (우선호가 10단계)": 10,
    "파생 체결 + 우선호가 (우선호가 5단계)": 5,
    "파생 체결 + 우선호가 (우선호가 10단계)": 10,
}

# 5단/10단이 갈리는 파생 호가 계열 **전부**. 주식선물 잘림을 풀 때 쓴다.
# 아직 디코더가 없는 R1(장운영TS+호가)·B2(Snapshot)도 같은 규칙을 탄다.
ALL_DEPTH_INTERFACE_NAMES = {
    **DEPTH_INTERFACE_NAMES,
    "파생 장운영TS + 우선호가 (우선호가 5단계)": 5,
    "파생 장운영TS + 우선호가 (우선호가 10단계)": 10,
    "파생 시세 Snapshot (우선호가 5단계)": 5,
    "파생 시세 Snapshot (우선호가 10단계)": 10,
}


def norm(s) -> str:
    return " ".join(str(s or "").split())


def toml_str(s: str) -> str:
    """TOML 기본 문자열. 개행은 공백으로 접는다 (엑셀 셀에 줄바꿈이 섞여 있다)."""
    return '"' + norm(s).replace("\\", "\\\\").replace('"', '\\"') + '"'


def load_product_groups(wb_channels, wb_spec) -> dict[str, str]:
    """3바이트 상품군 → 한글 이름.

    송신채널 v1.26 의 정보구분코드 시트가 최신이다 (`17F`/`18F`/`05S` 가 여기 있고
    정보분배 v1.341 별첨에는 없다). 별첨에만 있는 항목은 뒤에서 채운다.
    """
    out: dict[str, str] = {}
    for wb, sheet in ((wb_channels, "정보구분코드"), (wb_spec, "별첨-정보구분코드")):
        ws = wb[sheet]
        for row in ws.iter_rows(min_row=4, values_only=True):
            code, name = row[5], row[4]
            if not code or not name:
                continue
            code = str(code).strip()
            if len(code) == 3:
                out.setdefault(code, norm(name))
    return out


def parse_trcode_cell(cell: str) -> list[tuple[str, str | None, str | None]]:
    """'(파생A) DRV : G701F, G702F' → [(code, channel, market), ...].

    줄바꿈은 이어쓰기다: 괄호가 없으면 앞 줄의 회선을, 콜론이 없으면 앞 줄의 시장을
    물려받는다. 표준서가 셀 안에서 그렇게 접혀 있다.
    """
    out: list[tuple[str, str | None, str | None]] = []
    channel: str | None = None
    market: str | None = None
    for line in str(cell).split("\n"):
        line = line.strip()
        if not line or line == "-":
            continue
        m = LINE_RE.match(line)
        if not m:
            continue
        if m.group("grp"):
            channel = m.group("grp").strip()
        if m.group("mkt"):
            market = m.group("mkt").strip()
        for tok in re.split(r"[,/]", m.group("codes")):
            tok = tok.strip()
            if CODE_RE.match(tok):
                out.append((tok, channel, market))
    return out


def load_channel_codes(wb) -> dict[str, set[str]]:
    """전용선 시트: 인터페이스 이름 → trcode 집합.

    `제공정보`(이름)와 `제공정보 코드` 두 컬럼이 줄 단위로 짝을 이룬다.
    IP·포트 컬럼은 읽지 않는다.
    """
    ws = wb["전용선(UDP) 시세상품별 회선별 송신채널정보"]
    out: dict[str, set[str]] = {}
    for row in ws.iter_rows(values_only=True):
        names = str(row[CH_NAMES] or "").split("\n")
        codes = str(row[CH_CODES] or "").split("\n")
        for name, code_cell in zip(names, codes):
            name = norm(name)
            if not name:
                continue
            for tok in re.split(r"[,/]", code_cell):
                tok = tok.strip()
                if CODE_RE.match(tok):
                    out.setdefault(name, set()).add(tok)
    return out


def split_codes(
    seen: dict[str, list[dict]], interfaces: dict[str, dict], warnings: list[str]
) -> tuple[dict[str, dict], dict[str, dict]]:
    """trcode → 인터페이스를 확정 테이블과 회선별 테이블로 가른다.

    **trcode 는 인터페이스에 1:1 이 아니다.** 표준서에 세 가지 형태로 나온다:

    1. 회선마다 다른 전문 — `B201S` 는 증권A 회선에서 685B `증권 Snapshot`,
       주식파생 기초자산 회선에서 136B. 5바이트만 보고는 못 고른다.
       → `[code_by_channel]`. 회선을 모르면 디코드하면 안 된다.
    2. 같은 전문, 다른 시각 — `H101F` 는 IFMSSTD0005(장중 30초 주기)와
       IFMSRID0007(확정치 07:30/16:30) 양쪽에 있고 **길이가 같다**.
       레이아웃이 같으니 디코드는 하나로 되고, 의미만 시각으로 갈린다.
       → `[code]` + `also`.
    3. 전문 안의 구분자 — `M200G / 1`(시세) vs `/ 2`(종가). 길이 같음. 2와 같이 다룬다.
    """
    codes: dict[str, dict] = {}
    by_channel: dict[str, dict] = {}

    for code, entries in seen.items():
        # 주식선물 계열은 5단·10단 양쪽에 등록돼 있지만 실제로는 5단만 온다.
        # 길이가 갈리는 것으로 두면 아래에서 회선별 테이블로 떨어지는데, 회선이
        # 판별자가 아니므로 여기서 먼저 5단으로 확정한다.
        if code[2:] in TRUNCATED_TO_FIVE:
            depthful = [
                e for e in entries
                if ALL_DEPTH_INTERFACE_NAMES.get(interfaces[e["interface"]]["name"]) is not None
            ]
            if len(depthful) == len(entries) and depthful:
                entries = [
                    e for e in entries
                    if ALL_DEPTH_INTERFACE_NAMES[interfaces[e["interface"]]["name"]] == 5
                ]

        uniq: list[dict] = []
        for e in entries:
            if e not in uniq:
                uniq.append(e)
        ids: list[str] = []
        for e in uniq:
            if e["interface"] not in ids:
                ids.append(e["interface"])

        if len(ids) == 1:
            codes[code] = {
                "interface": ids[0],
                "group": code[2:],
                "channel": uniq[0]["channel"],
                "market": uniq[0]["market"],
                "also": [],
                "source": uniq[0]["source"],
            }
            continue

        lengths = {interfaces[i]["length"] for i in ids}
        if len(lengths) == 1:
            # 같은 레이아웃. 디코더는 하나, 의미는 시각·전문 내용으로 갈린다.
            codes[code] = {
                "interface": ids[0],
                "group": code[2:],
                "channel": uniq[0]["channel"],
                "market": uniq[0]["market"],
                "also": ids[1:],
                "source": uniq[0]["source"],
            }
            continue

        # 길이가 다르다 — 회선이 유일한 판별자다.
        mapping: dict[str, str] = {}
        for e in uniq:
            ch = e["channel"]
            if not ch:
                warnings.append(f"{code}: 길이가 갈리는데 회선이 비어 있다 ({e['interface']})")
                continue
            if ch in mapping and mapping[ch] != e["interface"]:
                warnings.append(
                    f"{code}: 같은 회선 {ch} 에 길이가 다른 전문 두 개 "
                    f"({mapping[ch]}, {e['interface']}) — 5바이트로 못 가른다"
                )
                continue
            mapping[ch] = e["interface"]
        by_channel[code] = {"group": code[2:], "by": mapping}

    return codes, by_channel


def derivative_depths(name_to_codes: dict[str, set[str]]) -> dict[int, set[str]]:
    """'파생 우선호가 (우선호가 N단계)' 계열에서 상품군별 단수를 뽑는다."""
    depths: dict[int, set[str]] = {5: set(), 10: set()}
    for name, depth in DEPTH_INTERFACE_NAMES.items():
        for code in name_to_codes.get(name, ()):
            depths[depth].add(code[2:])
    return depths


def main() -> int:
    for path in (SPEC, CHANNELS):
        if not path.exists():
            sys.exit(f"표준서가 없다: {path}")

    wb = openpyxl.load_workbook(SPEC, read_only=True, data_only=True)
    wb_ch = openpyxl.load_workbook(CHANNELS, read_only=True, data_only=True)
    groups = load_product_groups(wb_ch, wb)
    name_to_codes = load_channel_codes(wb_ch)
    ws = wb["인터페이스목록"]

    interfaces: dict[str, dict] = {}
    name_to_id: dict[str, list[str]] = {}
    seen: dict[str, list[dict]] = {}
    warnings: list[str] = []

    for row in ws.iter_rows(min_row=FIRST_DATA_ROW, values_only=True):
        iid = row[COL_ID]
        if not iid or not str(iid).strip().startswith("IF"):
            continue
        iid = str(iid).strip()
        length = row[COL_LENGTH]
        if not isinstance(length, int):
            warnings.append(f"{iid}: 길이가 정수가 아니다 ({length!r}) — 건너뛴다")
            continue

        name = norm(row[COL_NAME])
        interfaces[iid] = {
            "name": name,
            "length": length,
            "period": norm(row[COL_PERIOD]),
            "market": "/".join(str(row[COL_MARKET] or "").split()),
        }
        name_to_id.setdefault(name, []).append(iid)

        for code, channel, market in parse_trcode_cell(row[COL_TRCODE] or ""):
            seen.setdefault(code, []).append(
                {
                    "interface": iid,
                    "channel": channel or "",
                    "market": market or "",
                    "source": SPEC_VERSION,
                }
            )

    # 송신채널 v1.26 이 더 최신이다. 이름으로 이어 붙여 빠진 코드를 채운다.
    added = 0
    for name, codes in sorted(name_to_codes.items()):
        ids = name_to_id.get(name)
        if not ids:
            continue  # 정보분배 표준서에 없는 이름 (지수·인터넷 계열). 지어내지 않는다.
        if len(ids) > 1:
            warnings.append(f"{name!r}: 인터페이스가 {ids} 로 갈려 v1.26 코드를 못 붙인다")
            continue
        iid = ids[0]
        known = {e["interface"] for c in codes for e in seen.get(c, [])}
        for code in sorted(codes):
            if iid in {e["interface"] for e in seen.get(code, [])}:
                continue
            seen.setdefault(code, []).append(
                {
                    "interface": iid,
                    "channel": "",
                    "market": interfaces[iid]["market"].split("/")[0],
                    "source": CHANNELS_VERSION,
                }
            )
            added += 1
        del known

    for code in seen:
        if code[2:] not in groups:
            warnings.append(f"{code}: 상품군 {code[2:]} 이 정보구분코드에 없다")

    codes, by_channel = split_codes(seen, interfaces, warnings)
    depths = derivative_depths(name_to_codes)
    depths[10].discard("04F")  # 주식선물: 10단 등록이지만 5단만 온다
    depths[5].add("04F")
    if depths[10] != TEN_DEEP_EXPECTED:
        warnings.append(
            f"파생 호가 10단 상품군이 바뀌었다: {sorted(depths[10])} "
            f"(기대 {sorted(TEN_DEEP_EXPECTED)}) — jeed-krx 의 depth_for() 를 고칠 것"
        )

    # 데이터구분 2바이트 → 그 코드를 쓰는 인터페이스들. 표준서에 표가 없어서 역산한다.
    data_class: dict[str, list[str]] = {}

    def note(code: str, iid: str) -> None:
        bucket = data_class.setdefault(code[:2], [])
        if iid not in bucket:
            bucket.append(iid)

    for code, meta in codes.items():
        for iid in [meta["interface"], *meta["also"]]:
            note(code, iid)
    for code, meta in by_channel.items():
        for iid in meta["by"].values():
            note(code, iid)

    today = _dt.date.today().isoformat()
    lines: list[str] = [
        "# GENERATED — 손으로 고치지 않는다. tools/gen_krx_trcodes.py 를 다시 돌린다.",
        f"# 원본: {SPEC.name}",
        f"#       {CHANNELS.name}",
        "#",
        "# 두 표준서를 합친 표다. 길이는 정보분배 인터페이스목록에서, trcode 목록은 양쪽",
        "# 합집합에서 온다 — 새 상품은 송신채널 쪽에 먼저 뜬다 (17F 코스닥150 위클리옵션,",
        "# 18F 개별주식 위클리옵션은 정보분배 v1.341 에 아직 없다).",
        "#",
        "# 여기에 **멀티캐스트 IP·포트는 없다.** 송신채널 문서에 들어 있지만 읽지 않는다:",
        "# 회선 배정이라 표준서 값이 우리 회선에 그대로 오지 않는다. 배포 설정(conf/krx.toml)은",
        "# 사람이 적고, 기동 시 거기 적힌 trcode 를 이 표와 대조해 경고만 낸다.",
        "",
        f"spec_version = {toml_str(SPEC_VERSION)}",
        f"channel_spec_version = {toml_str(CHANNELS_VERSION)}",
        f"generated = {toml_str(today)}",
        f"interface_count = {len(interfaces)}",
        f"code_count = {len(codes)}",
        f"ambiguous_code_count = {len(by_channel)}",
        "",
        "# 정보구분+시장구분 3바이트 (trcode 의 뒤 3자리) → 상품군.",
        "# 멀티캐스트 그룹이 상품군마다 갈리므로, 전략이 안 보는 상품군은 소켓을 안 연다.",
        "[product_group]",
    ]
    for code in sorted(groups):
        lines.append(f'"{code}" = {toml_str(groups[code])}')

    lines += [
        "",
        "# 파생 호가 전문의 단수. 상품군이 정하고, 개별주식 계열만 10단이다.",
        "# jeed-krx 의 decode::derivative::{quote,trade_quote}::depth_for() 가 이 집합이다.",
        "[derivative_depth]",
        "five = [" + ", ".join(f'"{g}"' for g in sorted(depths[5])) + "]",
        "ten = [" + ", ".join(f'"{g}"' for g in sorted(depths[10])) + "]",
    ]

    lines += [
        "",
        "# 데이터구분 2바이트 (trcode 의 앞 2자리) → 이 구분을 쓰는 인터페이스.",
        "# 표준서에 표가 없어서 코드에서 역산했다.",
        "[data_class]",
    ]
    for dc in sorted(data_class):
        ids = ", ".join(toml_str(i) for i in sorted(data_class[dc]))
        names = " / ".join(interfaces[i]["name"] for i in sorted(data_class[dc]))
        lines.append(f"# {dc}: {names}")
        lines.append(f'"{dc}" = [{ids}]')

    lines += [
        "",
        "# 인터페이스 → 전문 길이. 길이는 인터페이스 단위로 고정이다.",
        "# 디코더는 길이 + 종료키워드(0xFF)를 먼저 보고 그 다음에 필드를 읽는다.",
        "[interface]",
    ]
    for iid in sorted(interfaces):
        it = interfaces[iid]
        lines.append(
            f'{iid} = {{ length = {it["length"]}, market = {toml_str(it["market"])}, '
            f'period = {toml_str(it["period"])}, name = {toml_str(it["name"])} }}'
        )

    lines += [
        "",
        "# trcode → 인터페이스. 길이는 [interface] 에서 끌어온다 (중복 저장하지 않는다).",
        "# channel 은 표준서의 회선 그룹(증권A/파생A/…)이지 우리 회선이 아니다.",
        "# source 는 이 코드가 어느 표준서에서 왔는지 — 1.26 만 있는 코드는 정보분배",
        "# 표준서가 아직 못 따라온 신규 상품이다.",
        "[code]",
    ]
    for code in sorted(codes):
        c = codes[code]
        also = ""
        if c["also"]:
            ids = ", ".join(toml_str(i) for i in c["also"])
            also = f", also = [{ids}]"
        lines.append(
            f'{code} = {{ interface = {toml_str(c["interface"])}, '
            f'group = "{c["group"]}", channel = {toml_str(c["channel"])}, '
            f'market = {toml_str(c["market"])}, source = {toml_str(c["source"])}{also} }}'
        )

    lines += [
        "",
        "# **같은 trcode 가 회선마다 다른 전문으로 온다.** 5바이트만 보고는 못 고른다 —",
        "# 예: B201S 는 증권A 회선에서 685B, 주식파생 기초자산 회선에서 136B.",
        "# 이 코드들은 [code] 에 없다. 어느 회선에서 받았는지를 같이 넘겨야 한다.",
        "[code_by_channel]",
    ]
    for code in sorted(by_channel):
        c = by_channel[code]
        pairs = ", ".join(f"{toml_str(k)} = {toml_str(v)}" for k, v in sorted(c["by"].items()))
        lines.append(f'{code} = {{ group = "{c["group"]}", by = {{ {pairs} }} }}')

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(lines) + "\n", encoding="utf-8")

    print(
        f"{OUT.relative_to(ROOT)}: 인터페이스 {len(interfaces)}, "
        f"trcode {len(codes)} (+회선별 {len(by_channel)}), v{CHANNELS_VERSION} 에서 {added} 개 보충"
    )
    print(f"  파생 호가 10단 상품군: {sorted(depths[10])}")
    for w in warnings:
        print(f"  경고: {w}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
