#!/usr/bin/env python3
"""Checks the database migrations against two rules.

  1. No two files share a number. Each phase takes the next free number on main. When two branches
     take the same one, the second to merge fails here and renumbers its file before it merges.
  2. Every file is registered in the server's migration list, and every registered name has a file.
     A file that is not registered never runs.

Usage: check-migrations.py [repository root]. Exit 0 when both rules hold, 1 otherwise.
Standard library only.
"""

import re
import sys
from collections import defaultdict
from pathlib import Path

NAME = re.compile(r"^(\d{4})_[a-z0-9_]+\.sql$")
REGISTERED = re.compile(r'\(\s*"(\d{4}_[a-z0-9_]+\.sql)"\s*,')
MIGRATIONS = Path("crates/scaffoldry-core/migrations")
REGISTRY = Path("crates/scaffoldry-server/src/repository.rs")


def problems(root):
    root = Path(root)
    files = sorted(p.name for p in (root / MIGRATIONS).glob("*.sql"))
    out = []
    by_number = defaultdict(list)
    for f in files:
        m = NAME.match(f)
        if not m:
            out.append(f"{f}: the name must be four digits, an underscore, and a lowercase slug")
            continue
        by_number[m.group(1)].append(f)
    for number, names in sorted(by_number.items()):
        if len(names) > 1:
            out.append(f"number {number} is used by {', '.join(names)}. Renumber the newer file to the next free number")
    registered = set(REGISTERED.findall((root / REGISTRY).read_text()))
    for f in files:
        if NAME.match(f) and f not in registered:
            out.append(f"{f} is not in the migration list in {REGISTRY}, so it never runs")
    for r in sorted(registered - set(files)):
        out.append(f"{REGISTRY} lists {r}, which is not in {MIGRATIONS}")
    return out


def main(argv):
    root = argv[1] if len(argv) > 1 else "."
    found = problems(root)
    for p in found:
        print(f"migration check: {p}")
    if not found:
        print("migration check: every number is used once and every file is registered")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
