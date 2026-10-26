#!/usr/bin/env python3
"""
Finds direct dependencies that have stopped being maintained, so a person decides what to do before one breaks us.

A dependency is read from every Cargo.toml and package.json in the repository, outside build output. Its newest
release is read from its own registry: crates.io for a crate, the npm registry for a package.

  frozen   the newest release is deprecated (npm) or yanked (crates.io), or there has been no release in 730 days
  stale    no release in 365 days, or in the tighter limit STALE_DAYS_FOR names for that package
  unknown  the registry did not answer, or named no release
  ok       anything else

A stale or frozen dependency is a review item, not a defect. Some small libraries are finished and rightly quiet.

Usage: audit-dependency-health.py [--all] [repository root]
Prints one line per dependency that is not ok (every line with --all). Exit 0 when all are ok, 1 otherwise.
It reads the network and its answer changes with the date, so it is not a CI gate. Run it on a schedule.
Standard library only. The pure functions are unit tested in test_audit_dependency_health.py.
"""

import json
import sys
import time
import tomllib
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

STALE_DAYS = 365
FROZEN_DAYS = 730
# A tighter stale limit for a young dependency, by name. declarative-pages.md phase 1 adds the OpenUI packages here.
STALE_DAYS_FOR = {}

SKIP_DIRS = {"node_modules", "target", ".git", "dist"}
CARGO_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")
NPM_TABLES = ("dependencies", "devDependencies")
# crates.io asks every client to name itself and to send at most one request a second.
USER_AGENT = "scaffoldry-dependency-health (https://github.com/scaffoldry-io/scaffoldry)"


def manifests(root, name):
    for p in sorted(Path(root).rglob(name)):
        if not SKIP_DIRS.intersection(p.relative_to(root).parts):
            yield p


def _crate_names(table):
    """Registry crates in one dependency table. A path, git, or workspace entry is not one."""
    for key, spec in (table or {}).items():
        if isinstance(spec, str):
            yield key
        elif isinstance(spec, dict) and not ({"path", "git", "workspace"} & spec.keys()):
            yield spec.get("package", key)


def direct_dependencies(root):
    """Sorted (ecosystem, name) pairs for every direct dependency, each once."""
    found = set()
    for p in manifests(root, "Cargo.toml"):
        doc = tomllib.loads(p.read_text())
        tables = [doc.get(t) for t in CARGO_TABLES] + [doc.get("workspace", {}).get("dependencies")]
        for table in tables:
            found.update(("crates", n) for n in _crate_names(table))
    for p in manifests(root, "package.json"):
        doc = json.loads(p.read_text())
        for t in NPM_TABLES:
            for name, spec in (doc.get(t) or {}).items():
                if not str(spec).startswith(("workspace:", "file:", "link:")):
                    found.add(("npm", name))
    return sorted(found)


def _when(text):
    return datetime.fromisoformat(text.replace("Z", "+00:00")) if text else None


def npm_facts(doc):
    """(version, released, deprecated message, yanked) for the `latest` tag of an npm registry document."""
    version = doc.get("dist-tags", {}).get("latest")
    manifest = doc.get("versions", {}).get(version, {})
    return version, _when(doc.get("time", {}).get(version)), manifest.get("deprecated"), False


def crate_facts(doc):
    """(version, released, deprecated message, yanked) for the newest stable version of a crates.io document."""
    version = doc.get("crate", {}).get("max_stable_version")
    versions = doc.get("versions", [])
    if version is None:
        return None, None, None, bool(versions) and all(v.get("yanked") for v in versions)
    entry = next((v for v in versions if v.get("num") == version), {})
    return version, _when(entry.get("created_at")), None, bool(entry.get("yanked"))


def classify(released, deprecated, yanked, now, stale_days=STALE_DAYS):
    """(status, reason) for one dependency."""
    if deprecated:
        return "frozen", f"the newest release is deprecated: {deprecated}"
    if yanked:
        return "frozen", "the newest release is yanked"
    if released is None:
        return "unknown", "the registry names no release"
    age = (now - released).days
    if age >= FROZEN_DAYS:
        return "frozen", f"no release in {age} days"
    if age >= stale_days:
        return "stale", f"no release in {age} days"
    return "ok", f"released {age} days ago"


def fetch(ecosystem, name):
    if ecosystem == "npm":
        url = "https://registry.npmjs.org/" + urllib.parse.quote(name, safe="@")
    else:
        url = "https://crates.io/api/v1/crates/" + urllib.parse.quote(name)
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT, "Accept": "application/json"})
    with urllib.request.urlopen(request, timeout=60) as response:
        doc = json.load(response)
    return npm_facts(doc) if ecosystem == "npm" else crate_facts(doc)


def main(argv):
    show_all = "--all" in argv[1:]
    paths = [a for a in argv[1:] if not a.startswith("--")]
    root = Path(paths[0]) if paths else Path(__file__).resolve().parents[2]
    now = datetime.now(timezone.utc)
    counts = {"ok": 0, "stale": 0, "frozen": 0, "unknown": 0}
    for ecosystem, name in direct_dependencies(root):
        try:
            version, released, deprecated, yanked = fetch(ecosystem, name)
            status, reason = classify(released, deprecated, yanked, now, STALE_DAYS_FOR.get(name, STALE_DAYS))
        except Exception as e:  # one registry failure is reported for that package, and the rest still run
            version, status, reason = None, "unknown", f"the registry did not answer: {e}"
        if ecosystem == "crates":
            time.sleep(1)
        counts[status] += 1
        if show_all or status != "ok":
            print(f"{status:8} {ecosystem:6} {name} {version or '-'}: {reason}")
    print(", ".join(f"{n} {s}" for s, n in counts.items()))
    return 0 if counts["ok"] == sum(counts.values()) else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
