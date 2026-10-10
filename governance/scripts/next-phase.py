#!/usr/bin/env python3
"""
Reads the run order in docs/plans/README.md and answers two questions for the build agent:

  next-phase.py next
      Prints "<brief> <phase>" for the first row that is not COMPLETED.
      Exit 0 when it can be started, 3 when it needs a person, 4 when nothing is left.

  next-phase.py check <brief> <phase> [--allow-out-of-order]
      Exit 0 when that row may be started. 2 unknown row, 3 blocked, 5 already completed,
      6 not the next open row.

A row is blocked when its text says "Needs ..." (a file, a dependency, or a decision that
only a person can supply) or when its phase is not a plain phase number such as "open phases".
Standard library only.
"""

import re
import sys
from dataclasses import dataclass
from pathlib import Path

OK, UNKNOWN, BLOCKED, NONE_LEFT, DONE, OUT_OF_ORDER = 0, 2, 3, 4, 5, 6
PLAIN_PHASE = re.compile(r"\d+[a-z]?(-\d+[a-z]?)?(,\d+[a-z]?)*")
LINK = re.compile(r"\[([a-z0-9-]+)\.md\]")


@dataclass
class Row:
    n: int
    brief: str
    phase: str
    text: str
    status: str


def norm_phase(raw):
    return re.sub(r"\s+", "", raw.replace("–", "-").replace("—", "-")).lower()


def parse_rows(text):
    rows = []
    for line in text.splitlines():
        line = line.strip()
        if not line.startswith("|"):
            continue
        cells = [c.strip() for c in line.strip("|").split("|")]
        if len(cells) < 4 or not cells[0].isdigit():
            continue
        m = LINK.match(cells[1])
        if not m:
            continue
        status = cells[4] if len(cells) > 4 else ""
        rows.append(Row(int(cells[0]), m.group(1), norm_phase(cells[2]), cells[3], status))
    return rows


def is_done(row):
    return "completed" in row.status.lower()


def first_open(rows):
    return next((r for r in rows if not is_done(r)), None)


def blocker(row):
    if not PLAIN_PHASE.fullmatch(row.phase):
        return f"phase '{row.phase}' is not a single phase and needs a person to scope it"
    if re.search(r"\bNeeds\b", row.text):
        return f"row {row.n} needs a person: {row.text}"
    return None


def check(rows, brief, phase, strict=True):
    """Returns (code, message)."""
    phase = norm_phase(phase)
    row = next((r for r in rows if r.brief == brief and r.phase == phase), None)
    if row is None:
        return UNKNOWN, f"no row for {brief} phase {phase}"
    if is_done(row):
        return DONE, f"row {row.n} ({brief} {phase}) is already COMPLETED"
    why = blocker(row)
    if why:
        return BLOCKED, why
    first = first_open(rows)
    if strict and first is not None and first.n != row.n:
        return OUT_OF_ORDER, f"row {first.n} ({first.brief} {first.phase}) comes first. Row {row.n} is out of order"
    return OK, f"row {row.n}: {row.text}"


def main(argv):
    args = [a for a in argv[1:] if not a.startswith("--")]
    flags = [a for a in argv[1:] if a.startswith("--")]
    readme = Path(__file__).resolve().parents[2] / "docs" / "plans" / "README.md"
    for f in flags:
        if f.startswith("--readme="):
            readme = Path(f.split("=", 1)[1])
    rows = parse_rows(readme.read_text(encoding="utf-8"))
    if not args:
        print(__doc__, file=sys.stderr)
        return 1
    if args[0] == "next":
        row = first_open(rows)
        if row is None:
            print("no open phases")
            return NONE_LEFT
        why = blocker(row)
        if why:
            print(f"BLOCKED: {why}")
            return BLOCKED
        print(f"{row.brief} {row.phase}")
        return OK
    if args[0] == "check" and len(args) == 3:
        code, msg = check(rows, args[1], args[2], strict="--allow-out-of-order" not in flags)
        print(msg)
        return code
    print(__doc__, file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
