#!/usr/bin/env python3
"""
Reads the run order in docs/plans/README.md and answers two questions for the build agent:

  next-phase.py next
      Prints "<brief> <phase>" for the first row that is not COMPLETED.
      Exit 0 when it can be started, 3 when it needs a person, 4 when nothing is left.

  next-phase.py check <brief> <phase> [--allow-out-of-order]
      Exit 0 when that row may be started. 2 unknown row, 3 blocked, 5 already completed,
      6 not the next open row.

Two agents at once. A claim is a pushed branch named feat/<brief>-phase-<n> (or agent/...).
A branch can be created on the remote only once, so the push is the lock.

  next-phase.py claims
      Lists the claims that are live. A claim on a COMPLETED row is ignored.
  next-phase.py available
      Prints "<brief> <phase>" for the first open row nobody holds and that no live claim
      stands in front of. Exit 0, 4 nothing left, 3 the next row needs a person,
      7 every row in reach is held or waiting. Reads the remote, so it fetches first.
  next-phase.py claim <brief> <phase>
      Re-checks the row, then creates feat/<brief>-phase-<n> on the remote from origin/main
      with one empty claim commit. Work on that branch.
      Exit 0 claimed, 7 refused, 2 unknown row, 5 already completed.
  next-phase.py release <brief> <phase>
      Deletes the claim branch. Use it when you stop without a pull request.

A claim only knows about the same brief. A phase that needs another brief's in-flight phase
is named in its brief's header. Read that line before you claim. Add --remote=<name> to use a
remote other than origin.

A row is blocked when its text says "Needs ..." (a file, a dependency, or a decision that
only a person can supply) or when its phase is not a plain phase number such as "open phases".
Standard library only.
"""

import re
import subprocess
import sys
import uuid
from dataclasses import dataclass
from pathlib import Path

OK, UNKNOWN, BLOCKED, NONE_LEFT, DONE, OUT_OF_ORDER, HELD = 0, 2, 3, 4, 5, 6, 7
WINDOW = 4  # open rows an agent may look ahead. Two agents never need more.
CLAIM_REF = re.compile(r"^refs/heads/(?:feat|agent)/([a-z0-9-]+?)-phase-([0-9a-z]+(?:-[0-9a-z]+)*)$")
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


def claim_branch(brief, phase):
    return f"feat/{brief}-phase-{phase}"


def claims_from_refs(refs):
    """Branch refs to a set of (brief, phase). Other branches are not claims."""
    found = set()
    for ref in refs:
        m = CLAIM_REF.match(ref)
        if m:
            found.add((m.group(1), norm_phase(m.group(2))))
    return found


def live_claims(rows, claims):
    """A leftover branch for a COMPLETED row holds nothing."""
    done = {(r.brief, r.phase) for r in rows if is_done(r)}
    return {c for c in claims if c not in done}


def claim_blocker(rows, row, claims):
    """Why this row cannot be claimed now, or None. `claims` holds live claims."""
    if is_done(row):
        return f"row {row.n} ({row.brief} {row.phase}) is already COMPLETED"
    if (row.brief, row.phase) in claims:
        return f"already claimed: {claim_branch(row.brief, row.phase)} exists"
    for brief, phase in sorted(claims):
        if brief == row.brief:
            return f"{brief} has work in flight ({claim_branch(brief, phase)}). One agent per brief"
    for earlier in rows:
        if earlier.brief == row.brief and earlier.n < row.n and not is_done(earlier):
            return f"{earlier.brief} phase {earlier.phase} (row {earlier.n}) is not COMPLETED. It must finish first"
    return blocker(row)


def next_available(rows, claims, window=WINDOW):
    """First open row an agent may take. Stops at a row that needs a person, because rows
    after it may depend on it. Looks at only `window` open rows."""
    open_rows = [r for r in rows if not is_done(r)][:window]
    for row in open_rows:
        if blocker(row):
            return None
        if claim_blocker(rows, row, claims) is None:
            return row
    return None


def remote_refs(remote):
    out = subprocess.run(
        ["git", "ls-remote", "--heads", remote, "feat/*-phase-*", "agent/*-phase-*"],
        check=True, capture_output=True, text=True,
    ).stdout
    return [line.split("\t", 1)[1] for line in out.splitlines() if "\t" in line]


def push_claim(remote, branch, sha):
    """Creates the branch on the remote, or returns False if it already exists.

    Each claim is its own empty commit on top of `sha`, with a random note. Two claimers push
    different commits, so git rejects the second as a non-fast-forward. Pushing the same commit
    twice would succeed silently, which is why the commit is unique.
    """
    commit = subprocess.run(
        ["git", "commit-tree", f"{sha}^{{tree}}", "-p", sha, "-m", f"chore: claim {branch}\n\nclaim {uuid.uuid4().hex}"],
        check=True, capture_output=True, text=True,
    ).stdout.strip()
    done = subprocess.run(["git", "push", remote, f"{commit}:refs/heads/{branch}"], capture_output=True, text=True)
    return done.returncode == 0


def run_claim_command(args, rows, remote):
    subprocess.run(["git", "fetch", "--quiet", remote], check=True)
    claims = live_claims(rows, claims_from_refs(remote_refs(remote)))
    if args[0] == "claims":
        for brief, phase in sorted(claims):
            print(claim_branch(brief, phase))
        return OK
    if args[0] == "available":
        if first_open(rows) is None:
            print("no open phases")
            return NONE_LEFT
        row = next_available(rows, claims)
        if row is None:
            why = blocker(first_open(rows))
            print(f"BLOCKED: {why}" if why else "every row in reach is held or waiting")
            return BLOCKED if why else HELD
        print(f"{row.brief} {row.phase}")
        return OK
    if len(args) != 3:
        print(__doc__, file=sys.stderr)
        return 1
    brief, phase = args[1], norm_phase(args[2])
    row = next((r for r in rows if r.brief == brief and r.phase == phase), None)
    if row is None:
        print(f"no row for {brief} phase {phase}")
        return UNKNOWN
    branch = claim_branch(brief, phase)
    if args[0] == "release":
        done = subprocess.run(["git", "push", remote, "--delete", branch], capture_output=True, text=True)
        print(f"released {branch}" if done.returncode == 0 else f"could not delete {branch}: {done.stderr.strip()}")
        return OK if done.returncode == 0 else HELD
    why = claim_blocker(rows, row, claims)
    if why:
        print(why)
        return DONE if is_done(row) else (BLOCKED if blocker(row) and why == blocker(row) else HELD)
    sha = subprocess.run(["git", "rev-parse", f"{remote}/main"], check=True, capture_output=True, text=True).stdout.strip()
    if not push_claim(remote, branch, sha):
        print(f"already claimed: {branch} was created first by someone else")
        return HELD
    print(f"claimed {branch}. Run: git fetch {remote} {branch} && git checkout -B {branch} {remote}/{branch}")
    return OK


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
    remote = next((f.split("=", 1)[1] for f in flags if f.startswith("--remote=")), "origin")
    if args[0] in ("claims", "available", "claim", "release"):
        return run_claim_command(args, rows, remote)
    if args[0] == "check" and len(args) == 3:
        code, msg = check(rows, args[1], args[2], strict="--allow-out-of-order" not in flags)
        print(msg)
        return code
    print(__doc__, file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
