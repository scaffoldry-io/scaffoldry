#!/usr/bin/env python3
"""
Checks the changes an automated build agent made on an `agent/<brief>-phase-<n>` branch.
It fails the build when the agent:

  1. touched CI, the dev container, AGENTS.md, LICENSE, or an existing governance script;
  2. changed a dependency manifest or lock file without the `deps-approved` label;
  3. edited the plans beyond the Status cell of its own row and the status line of its brief;
  4. removed tests, skipped tests, or added `#[ignore]` (except the million-row proof).

Usage:
  agent-guard.py --branch agent/guards-phase-1 [--base origin/main] [--head HEAD] [--deps-approved]

Standard library only. The pure functions are unit tested in test_agent_guard.py.
"""

import re
import subprocess
import sys

BRANCH = re.compile(r"^agent/([a-z0-9-]+?)-phase-([0-9a-z]+(?:-[0-9a-z]+)*)$")
DEP_FILE = re.compile(r"(^|/)(Cargo\.(toml|lock)|package(-lock)?\.json)$")
ANY_CHANGE_PROTECTED = (".github/", ".devcontainer/")
ANY_CHANGE_PROTECTED_FILES = ("AGENTS.md", "LICENSE")
EXISTING_SCRIPT_PREFIXES = (
    "governance/scripts/audit-",
    "governance/scripts/next-phase",
    "governance/scripts/agent-guard",
    "governance/scripts/test_",
)
TEST_PATH = re.compile(r"(^|/)(tests?/|[^/]*_test\.rs$|[^/]*\.test\.[jt]sx?$)")
TEST_MARKER = re.compile(r"#\[(tokio::)?test\]|\b(it|test)\s*\(")
SKIP_MARKER = re.compile(r"\b(it|test|describe)\.(skip|only)\s*\(|\bxit\s*\(|\bxdescribe\s*\(|\bfit\s*\(")


def parse_branch(branch):
    m = BRANCH.match(branch)
    return (m.group(1), m.group(2)) if m else None


def check_paths(changes, deps_approved):
    """changes: list of (status letter, path)."""
    bad = []
    for status, path in changes:
        if path.startswith(ANY_CHANGE_PROTECTED) or path in ANY_CHANGE_PROTECTED_FILES:
            bad.append(f"{path}: protected. Only a person changes this ({status})")
        elif path.startswith(EXISTING_SCRIPT_PREFIXES) and status != "A":
            bad.append(f"{path}: an existing governance script cannot be modified or deleted ({status})")
        elif DEP_FILE.search(path) and not deps_approved:
            bad.append(f"{path}: dependency change needs the 'deps-approved' label")
    return bad


def parse_changed_lines(diff_text):
    """Unified diff (-U0) to {file: (removed lines, added lines)}."""
    files, current = {}, None
    for line in diff_text.splitlines():
        if line.startswith("+++ b/"):
            current = line[6:]
            files.setdefault(current, ([], []))
        elif line.startswith(("+++ ", "--- ", "diff ", "@@", "index ")):
            continue
        elif current and line.startswith("-"):
            files[current][0].append(line[1:])
        elif current and line.startswith("+"):
            files[current][1].append(line[1:])
    return files


def _row_cells(line):
    return [c.strip() for c in line.strip().strip("|").split("|")]


def check_readme(removed, added, brief, phase):
    """Only the Status cell of the branch's own row may change."""
    if len(removed) != len(added):
        return ["docs/plans/README.md: lines were added or removed. Only a Status cell may change"]
    bad = []
    for old, new in zip(removed, added):
        if not (old.strip().startswith("|") and new.strip().startswith("|")):
            bad.append("docs/plans/README.md: a line outside the run-order table changed")
            continue
        a, b = _row_cells(old), _row_cells(new)
        if len(a) != len(b) or a[:-1] != b[:-1]:
            bad.append(f"docs/plans/README.md: more than the Status cell changed: {new.strip()[:80]}")
            continue
        link = re.match(r"\[([a-z0-9-]+)\.md\]", a[1]) if len(a) > 2 else None
        row_phase = re.sub(r"\s+", "", a[2].replace("–", "-")).lower() if len(a) > 2 else ""
        if not link or link.group(1) != brief or row_phase != phase:
            bad.append(f"docs/plans/README.md: this branch is for {brief} phase {phase}, not that row")
    return bad


def check_plan_edits(changed):
    bad = []
    for path, (removed, added) in changed.items():
        if not path.startswith("docs/plans/") or path == "docs/plans/README.md":
            continue
        for line in removed + added:
            if not line.lstrip().startswith("> **Status"):
                bad.append(f"{path}: only the '> **Status' line of a brief may change")
                break
    return bad


def check_tests(changed):
    bad = []
    for path, (removed, added) in changed.items():
        if not TEST_PATH.search(path):
            continue
        if sum(bool(TEST_MARKER.search(x)) for x in removed) > sum(bool(TEST_MARKER.search(x)) for x in added):
            bad.append(f"{path}: fewer tests than before. Tests may be replaced, never reduced")
        for line in added:
            if SKIP_MARKER.search(line):
                bad.append(f"{path}: a skipped or focused test was added: {line.strip()[:60]}")
        if any(l.strip() == "#[ignore]" for l in added) and not any("million_rows" in l for l in added):
            bad.append(f"{path}: #[ignore] is allowed only for the million-row proof")
    return bad


def git(*args):
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def main(argv):
    opts = {"--base": "origin/main", "--head": "HEAD", "--branch": ""}
    flags = set()
    it = iter(argv[1:])
    for a in it:
        if a in opts:
            opts[a] = next(it)
        else:
            flags.add(a)
    parsed = parse_branch(opts["--branch"])
    if not parsed:
        print(f"Not an agent branch: {opts['--branch']!r}. Nothing to check.")
        return 0
    brief, phase = parsed
    span = f"{opts['--base']}...{opts['--head']}"
    changes = []
    for line in git("diff", "--name-status", span).splitlines():
        parts = line.split("\t")
        changes.append((parts[0][0], parts[-1]))
    changed = parse_changed_lines(git("diff", "-U0", span))
    problems = check_paths(changes, "--deps-approved" in flags)
    if "docs/plans/README.md" in changed:
        problems += check_readme(*changed["docs/plans/README.md"], brief, phase)
    problems += check_plan_edits(changed)
    problems += check_tests(changed)
    if problems:
        print(f"Agent guard failed for {opts['--branch']} ({len(problems)} problem(s)):", file=sys.stderr)
        for p in problems:
            print(f"  - {p}", file=sys.stderr)
        return 1
    print(f"Agent guard passed for {opts['--branch']}: {len(changes)} file(s) changed, all within bounds.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
