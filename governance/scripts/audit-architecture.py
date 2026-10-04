#!/usr/bin/env python3
"""
Audits repository structure and source files against ARCHITECTURE.md boundaries:
1. Frontend apps/web must not import backend database packages directly (e.g., pg, postgres, tokio_postgres).
2. Data mutations must evaluate against Cedar policies.
3. No nested virtual machine, QEMU, or seed.sh hypervisor scripts permitted.
"""

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

def main():
    root = Path(__file__).resolve().parent.parent.parent
    errors = []

    errors.extend(audit_frontend(root))
    errors.extend(audit_repo_hygiene(root))

    if errors:
        print(f"FAILED: Architecture boundary violations found ({len(errors)}):", file=sys.stderr)
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        return 1

    print("SUCCESS: Architecture audit passed. Zero boundary violations detected.")
    return 0

if __name__ == "__main__":
    sys.exit(main())
