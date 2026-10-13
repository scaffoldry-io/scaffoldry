#!/usr/bin/env python3
"""Splits the Rust integration tests into a heavy group and a light group for CI.

  rust-test-groups.py heavy   Prints the cargo arguments that run only the heavy tests.
  rust-test-groups.py light   Prints the cargo arguments that run everything else.

The heavy tests are listed in governance/rust-heavy-tests.json. Every file under crates/*/tests
lands in exactly one group, so a new test is never left out. A name in the list with no file is an
error, so the list cannot drift.

To rebalance, read the "finished in" time of each test file in the CI log of both Rust jobs. Keep the
slowest files in the heavy list until the two jobs end within a minute of each other. Local times do
not predict CI times, so use the CI log.
"""

import json
import sys
from pathlib import Path


class GroupError(Exception):
    pass


def groups(root):
    root = Path(root)
    names = sorted({p.stem for p in (root / "crates").glob("*/tests/*.rs")})
    heavy = json.loads((root / "governance" / "rust-heavy-tests.json").read_text())["heavy"]
    missing = [h for h in heavy if h not in names]
    if missing:
        raise GroupError("rust-heavy-tests.json names tests that do not exist: " + ", ".join(missing))
    return sorted(heavy), [n for n in names if n not in heavy]


def cargo_args(root, group):
    heavy, light = groups(root)
    if group == "heavy":
        names, args = heavy, []
    elif group == "light":
        names, args = light, ["--lib", "--bins"]
    else:
        raise GroupError("the group is heavy or light")
    for n in names:
        args += ["--test", n]
    return args


def main(argv):
    if len(argv) != 2:
        print(__doc__)
        return 2
    try:
        print(" ".join(cargo_args(Path(__file__).resolve().parents[2], argv[1])))
    except GroupError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
