#!/usr/bin/env python3
"""
Audits repository structure and source files against ARCHITECTURE.md boundaries:
1. Frontend apps/web must not import backend database packages directly (e.g., pg, postgres, tokio_postgres).
2. Data mutations must evaluate against Cedar policies.
3. No nested virtual machine, QEMU, or seed.sh hypervisor scripts permitted.
4. Web screens must not call alert(), confirm(), or prompt(). Use ConfirmAction or Toast from the kit.
5. Web screens must not swallow an error with an empty catch. Show it with useAsync, ErrorState, or Toast.
6. The data crate is the single door to rows: SQL on the record, index and label tables lives in crates/scaffoldry-data.
"""

import re
import sys
from pathlib import Path

FORBIDDEN_FRONTEND_IMPORTS = [
    "from 'pg'",
    'from "pg"',
    "from 'postgres'",
    'from "postgres"',
    "require('pg')",
    'require("pg")',
    "require('postgres')",
    'require("postgres")',
    "import psycopg2",
    "import asyncpg",
]

FORBIDDEN_REPO_PATTERNS = [
    "qemu-system",
    "libvirt",
    "seed.sh",
]

def audit_frontend(root: Path) -> list[str]:
    errors = []
    web_dir = root / "apps" / "web"
    if not web_dir.exists():
        return errors

    for ext in ("*.ts", "*.tsx", "*.js", "*.jsx"):
        for source_file in web_dir.rglob(ext):
            if "node_modules" in source_file.parts or ".next" in source_file.parts:
                continue
            content = source_file.read_text(encoding="utf-8")
            for pattern in FORBIDDEN_FRONTEND_IMPORTS:
                if pattern in content:
                    errors.append(f"{source_file.relative_to(root)}: directly imports database driver ('{pattern}'). Must route through API & Cedar policy.")
    return errors

# A browser dialog call. A leading dot is a method of something else, except window.
DIALOG_CALL = re.compile(r"(?<![\w.$])(?:window\.)?(?:alert|confirm|prompt)\s*\(")
# A catch block, or a .catch handler, with nothing in it.
_EMPTY = r"\s*"
EMPTY_CATCH = re.compile(
    r"catch\s*(?:\([^)]*\))?\s*\{" + _EMPTY + r"\}"
    r"|\.catch\(\s*(?:async\s*)?\(\s*\w*\s*\)\s*=>\s*(?:\{" + _EMPTY + r"\}|undefined|void 0)\s*\)",
    re.S,
)

def strip_comments(text: str) -> str:
    """Blank out comments, keeping line numbers, so a word in a comment is not a call."""
    def blank(m):
        return re.sub(r"[^\n]", " ", m.group(0))
    return re.sub(r"/\*.*?\*/|(?<![:\"'])//[^\n]*", blank, text, flags=re.S)

def audit_web_screens(root: Path) -> list[str]:
    errors = []
    src = root / "apps" / "web" / "src"
    if not src.exists():
        return errors
    for ext in ("*.ts", "*.tsx"):
        for source_file in src.rglob(ext):
            if "test" in source_file.relative_to(src).parts[:1] or "ui" in source_file.relative_to(src).parts[:1]:
                continue
            content = source_file.read_text(encoding="utf-8")
            rel = source_file.relative_to(root)
            for m in DIALOG_CALL.finditer(strip_comments(content)):
                line = content.count("\n", 0, m.start()) + 1
                errors.append(f"{rel}:{line}: browser dialog '{m.group(0)}'. Use ConfirmAction or Toast from the kit.")
            for m in EMPTY_CATCH.finditer(content):
                line = content.count("\n", 0, m.start()) + 1
                errors.append(f"{rel}:{line}: empty catch swallows an error. Show it with useAsync, ErrorState, or Toast.")
    return errors

def audit_repo_hygiene(root: Path) -> list[str]:
    errors = []
    # Verify no VM hypervisor leftovers are reintroduced
    for script in root.rglob("*.sh"):
        if ".git" in script.parts or "node_modules" in script.parts:
            continue
        content = script.read_text(encoding="utf-8")
        for pattern in FORBIDDEN_REPO_PATTERNS:
            if pattern in content:
                errors.append(f"{script.relative_to(root)}: contains forbidden hypervisor pattern '{pattern}'. Scaffoldry uses plain container compose.")
    return errors

# The tables only the data crate (crates/scaffoldry-data) may read or write. See docs/plans/data-layer.md, "The boundary".
DATA_DOOR_TABLES = ("dataset_records", "record_index", "record_keys", "data_labels", "label_changes", "row_shares", "row_share_rows", "record_audience")
DATA_DOOR_CRATE = "crates/scaffoldry-data/"
# SQL text that reads or writes one of the tables. A bare word in prose or an identifier is not matched.
DATA_DOOR_SQL = re.compile(
    r"\b(?:FROM|JOIN|INTO|UPDATE|TABLE)[\s\\]+(" + "|".join(DATA_DOOR_TABLES) + r")\b", re.I
)

# Server files that still hold SQL on these tables. Each has a reason and a note on where it goes. A file not
# listed here fails the audit. Moving a file's SQL into the data crate means deleting its entry. Adding an entry
# needs a reviewer's agreement. Migrations are not .rs files, and integration tests under crates/*/tests are exempt:
# they seed rows and read them back to prove behavior, and they are not code that serves a request.
_S = "crates/scaffoldry-server/src/"
DATA_DOOR_ALLOWED = {
    "crates/scaffoldry-engine/src/access.rs": ("Compiles an access predicate to SQL text over record_index. It only builds a string and runs nothing.", "Move the compiler beside the readers that run it, when the scope builder moves."),
    _S + "repository.rs": ("Paged and filtered record reads, counts, bulk import writes and flag recomputation share one query builder with the access scope.", "Move the read queries with the scope builder in the access-model phase that reshapes it."),
    _S + "repository_index.rs": ("Builds index reads, aggregates and the batched index copy from dynamic SQL over record_index joined to dataset_records.", "Move the page, count and aggregate builders into the data crate as one unit."),
    _S + "repository_links.rs": ("Rollup SQL joins record_links to record_index and dataset_records.", "Move with the link reads when record_links joins the data crate."),
    _S + "repository_trash.rs": ("Trash list, count and batch restore read dataset_records.", "Move the trash reads next to soft_delete and restore."),
    _S + "repository_undo.rs": ("Undo reads a record's version and data to put it back.", "Move the two reads into records.rs."),
    _S + "repository_snapshots.rs": ("Snapshots copy and restore whole tables of records.", "Move the copy and restore statements into the data crate."),
    _S + "repository_comments.rs": ("One read of a record's version before a comment is stored.", "Replace with records::live_version."),
    _S + "calc_jobs.rs": ("The recalculation job pages through a table's rows and writes computed values.", "Move the paging reads into records.rs as a keyed page function."),
    _S + "index_jobs.rs": ("One count of a table's live rows for the index build progress.", "Replace with a records::count_live function."),
    _S + "content_scan.rs": ("The scanner pages through a table's rows for the sensitive content scan.", "Move onto the same keyed page function as calc_jobs."),
    _S + "export_jobs.rs": ("The export job pages through a table's rows.", "Move onto the same keyed page function as calc_jobs."),
    _S + "import_jobs.rs": ("The import job reads and writes batches of rows.", "Move the batch statements into records.rs."),
    _S + "purge.rs": ("Retention purge deletes rows, index rows and links, and strips a field from every row.", "Move the purge statements into the data crate with a hold-aware delete."),
    _S + "holds.rs": ("A legal hold reads the rows it covers.", "Move the row reads into records.rs."),
    _S + "webhooks.rs": ("A webhook reads one record's data to build its payload.", "Replace with a records::data function."),
    _S + "connections.rs": ("A connection's sync reads and writes rows.", "Move the statements into records.rs."),
    _S + "service/connections.rs": ("A connection's preview counts rows.", "Replace with records::count_live."),
    _S + "service/guest_admin.rs": ("The guest admin screen counts rows by form and submitter.", "Move the counts into records.rs."),
    _S + "service/guest_forms.rs": ("A guest form counts a submitter's rows.", "Move the count into records.rs."),
    _S + "service/proposals.rs": ("Proposal impact counts the rows an app change touches.", "Replace with records::count_live."),
    _S + "service/synthesize.rs": ("Synthetic sample rows check an id before insert.", "Replace with records::exists."),
}


def blank_comments(text: str) -> str:
    """Blank whole-line // comments, keeping line numbers, so prose that names a table is not read as SQL."""
    return "\n".join("" if line.lstrip().startswith("//") else line for line in text.split("\n"))


def data_door_hits(root: Path) -> dict:
    """Maps each .rs file outside the data crate and outside crates/*/tests to its SQL hits on the guarded tables."""
    hits = {}
    for source_file in (root / "crates").rglob("*.rs"):
        rel = source_file.relative_to(root)
        if "target" in rel.parts or "tests" in rel.parts[2:3]:
            continue
        posix = rel.as_posix()
        if posix.startswith(DATA_DOOR_CRATE):
            continue
        text = blank_comments(source_file.read_text(encoding="utf-8"))
        found = [(text.count("\n", 0, m.start()) + 1, m.group(1).lower()) for m in DATA_DOOR_SQL.finditer(text)]
        if found:
            hits[posix] = found
    return hits


def audit_data_door(root: Path) -> list[str]:
    """Any .rs file outside the data crate with SQL on the record, index or label tables fails, unless it is listed."""
    errors = []
    for posix, found in sorted(data_door_hits(root).items()):
        if posix in DATA_DOOR_ALLOWED:
            continue
        line, table = found[0]
        errors.append(
            f"{posix}:{line}: SQL on '{table}' outside crates/scaffoldry-data. Add a function to the data crate and call it."
        )
    return errors


def stale_data_door_entries(root: Path) -> list[str]:
    """Allow-list entries whose file no longer holds such SQL. They must be deleted, so the list only shrinks."""
    hits = data_door_hits(root)
    return sorted(p for p in DATA_DOOR_ALLOWED if p not in hits)


def main():
    root = Path(__file__).resolve().parent.parent.parent
    errors = []

    errors.extend(audit_frontend(root))
    errors.extend(audit_web_screens(root))
    errors.extend(audit_repo_hygiene(root))
    errors.extend(audit_data_door(root))
    errors.extend(f"{p}: the data-door allow-list names a file with no such SQL. Delete the entry." for p in stale_data_door_entries(root))

    if errors:
        print(f"FAILED: Architecture boundary violations found ({len(errors)}):", file=sys.stderr)
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        return 1

    print("SUCCESS: Architecture audit passed. Zero boundary violations detected.")
    return 0

if __name__ == "__main__":
    sys.exit(main())
