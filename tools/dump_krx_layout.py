#!/usr/bin/env python3
"""KRX 표준서 인터페이스정의서 → documents/krx/layouts.md.

    python tools/dump_krx_layout.py            # JEED_INTERFACES 를 문서로
    python tools/dump_krx_layout.py IFMSRPD0037 [...]   # 골라서 표준출력으로

**표준서의 오프셋 컬럼(`누적/길이`)은 끝 오프셋이다.** 시작으로 읽으면 필드가 한 칸씩
밀리고, 밀린 채로도 값이 나오기 때문에 조용히 틀린다 (한 번 당했다: G7 동적상하한가를
[163:172]로 읽어 상한 < 하한이 나왔다). 그래서 `start = cum - len` 을 여기서 한 번만
계산하고, 디코더는 사람이 센 숫자가 아니라 이 문서를 베낀다.
"""

from __future__ import annotations

import sys
from pathlib import Path

try:
    import openpyxl
except ImportError:  # pragma: no cover
    sys.exit("openpyxl 이 필요하다: pip install openpyxl")

ROOT = Path(__file__).resolve().parent.parent
SPEC = ROOT / "documents/krx/접속표준서(정보분배-UDP_TCP실시간)_v1.341-배포용.xlsx"
OUT = ROOT / "documents/krx/layouts.md"

# 인터페이스정의서 컬럼 (0-based)
C_ID, C_NAME, C_SEQ, C_ITEM, C_DECIMALS, C_TYPE, C_LEN, C_CUM, C_NOTE = 0, 1, 3, 4, 6, 7, 8, 9, 12

# Jeed 가 디코드하는 것들. 작업 순서는 documents/todo.md §6.
JEED_INTERFACES = [
    ("IFMSRPD0034", "B6 파생 우선호가 5단"),
    ("IFMSRPD0035", "B6 파생 우선호가 10단 (주식선물·주식옵션)"),
    ("IFMSRPD0037", "G7 파생 체결+우선호가 5단"),
    ("IFMSRPD0038", "G7 파생 체결+우선호가 10단"),
    ("IFMSRPD0036", "A3 파생 체결"),
    ("IFMSRPD0043", "V1 파생 가격제한폭확대발동"),
    ("IFMSRPD0042", "Q2 파생 동적상하한가 적용 및 해제"),
    ("IFMSRPD0019", "M4 장운영스케줄공개"),
    ("IFMSRPD0002", "B6 주식 우선호가 (MM/LP호가 제외)"),
    ("IFMSRPD0003", "B7 ETF·ELW·ETN 우선호가 (MM/LP호가 포함)"),
    ("IFMSRPD0004", "A3 증권 체결 (주식·ETF 공통)"),
    ("IFMSRPD0023", "B6 일반채권·국고채권 우선호가"),
    ("IFMSRPD0027", "A3 채권 체결"),
    ("IFMSRPD0029", "G7 일반채권·국고채권 체결 + 우선호가"),
    ("IFMSRPD0024", "B6 소액채권 우선호가 (레벨마다 채권종류 블록이 따라온다)"),
    ("IFMSRPD0030", "G7 소액채권 체결 + 우선호가"),
]


def load(wb) -> dict[str, dict]:
    ws = wb["인터페이스정의서"]
    out: dict[str, dict] = {}
    for row in ws.iter_rows(min_row=3, values_only=True):
        iid = row[C_ID]
        if not iid or not str(iid).strip().startswith("IF"):
            continue
        iid = str(iid).strip()
        length, cum = row[C_LEN], row[C_CUM]
        if not isinstance(length, int) or not isinstance(cum, int):
            continue
        entry = out.setdefault(iid, {"name": " ".join(str(row[C_NAME] or "").split()), "fields": []})
        entry["fields"].append(
            {
                "seq": str(row[C_SEQ] or "").strip(),
                "item": " ".join(str(row[C_ITEM] or "").split()),
                "type": str(row[C_TYPE] or "").strip(),
                "decimals": row[C_DECIMALS],
                "len": length,
                "start": cum - length,
                "end": cum,
                "note": " ".join(str(row[C_NOTE] or "").split()),
            }
        )
    return out


def render(iid: str, entry: dict, blurb: str = "") -> list[str]:
    fields = entry["fields"]
    total = fields[-1]["end"] if fields else 0

    lines = [f"### {iid} — {entry['name']}"]
    if blurb:
        lines.append("")
        lines.append(f"`{blurb}`")
    lines += ["", f"전문 길이 **{total}B**, 필드 {len(fields)}개.", ""]

    # 시작 오프셋이 이어지지 않으면 표준서를 잘못 읽은 것이다.
    gaps = [
        f"{a['item']}[{a['end']}] → {b['item']}[{b['start']}]"
        for a, b in zip(fields, fields[1:])
        if a["end"] != b["start"]
    ]
    if gaps:
        lines += ["> ⚠️ 오프셋이 이어지지 않는다: " + ", ".join(gaps), ""]

    lines += ["| # | 항목 | 타입 | 소수 | 길이 | `[start:end]` | 비고 |", "|--:|---|---|--:|--:|---|---|"]
    for f in fields:
        note = f["note"].replace("|", "\\|")[:90]
        dec = f["decimals"] if isinstance(f["decimals"], int) else ""
        lines.append(
            f'| {f["seq"]} | {f["item"]} | {f["type"]} | {dec} | {f["len"]} '
            f'| `[{f["start"]}:{f["end"]}]` | {note} |'
        )
    lines.append("")
    return lines


def main(argv: list[str]) -> int:
    if not SPEC.exists():
        sys.exit(f"표준서가 없다: {SPEC}")
    wb = openpyxl.load_workbook(SPEC, read_only=True, data_only=True)
    defs = load(wb)

    if argv:
        for iid in argv:
            if iid not in defs:
                print(f"{iid}: 없다", file=sys.stderr)
                continue
            print("\n".join(render(iid, defs[iid])))
        return 0

    lines = [
        "# KRX 전문 레이아웃 (Jeed 가 디코드하는 것)",
        "",
        "GENERATED — 손으로 고치지 않는다. `tools/dump_krx_layout.py` 를 다시 돌린다.",
        "원본: 접속표준서(정보분배-UDP_TCP실시간) v1.341 인터페이스정의서.",
        "",
        "> **표준서의 `누적/길이` 컬럼은 끝 오프셋이다.** 여기 `[start:end]` 는 그걸",
        "> `start = 누적 - 길이` 로 풀어 놓은 것이다. 시작 오프셋으로 착각하면 필드가 한 칸씩",
        "> 밀리는데, 밀린 채로도 값이 나오기 때문에 조용히 틀린다. **디코더는 사람이 센 숫자가",
        "> 아니라 이 표를 베낀다.**",
        "",
        "> 전문 끝은 `0xFF`(정보분배메세지종료키워드)이고 길이는 전문마다 고정이다.",
        "> **길이 + 종료키워드를 먼저 보고** 그 다음에 필드를 읽는다.",
        "",
    ]
    for iid, blurb in JEED_INTERFACES:
        if iid not in defs:
            print(f"경고: {iid} 가 정의서에 없다", file=sys.stderr)
            continue
        lines += render(iid, defs[iid], blurb)

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(lines), encoding="utf-8")
    print(f"{OUT.relative_to(ROOT)}: 인터페이스 {len(JEED_INTERFACES)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
