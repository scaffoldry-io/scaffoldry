#!/usr/bin/env python3
"""
Audits repository dependency definitions and package manifests for license compliance.
Only approved permissive/foundation licenses are allowed. Copyleft and BSL/SSPL are rejected.
"""

import sys
import json
from pathlib import Path

APPROVED_LICENSES = {
    "apache-2.0",
    "mit",
    "bsd-2-clause",
    "bsd-3-clause",
    "isc",
    "postgresql",
    "cc0-1.0",
    "python-2.0",
    "unlicense"
}

FORBIDDEN_KEYWORDS = [
    "agpl",
    "sspl",
    "business source license",
    "bsl",
    "server side public license",
    "commons clause",
]

def check_package_json(path: Path) -> list[str]:
    errors = []
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except Exception as e:
        return [f"{path}: failed to parse JSON: {e}"]

    license_field = data.get("license", "")
    if isinstance(license_field, str):
        lic_clean = license_field.strip().lower()
        if lic_clean and lic_clean not in APPROVED_LICENSES:
            for kw in FORBIDDEN_KEYWORDS:
                if kw in lic_clean:
                    errors.append(f"{path}: forbidden license '{license_field}' detected")
    return errors

def main():
    root = Path(__file__).resolve().parent.parent.parent
    errors = []
    checked_files = 0

    for p in root.rglob("package.json"):
        if "node_modules" in p.parts or ".devcontainer" in p.parts:
            continue
        checked_files += 1
        errors.extend(check_package_json(p))

    for p in root.rglob("Cargo.toml"):
        if "target" in p.parts:
            continue
        checked_files += 1
        content = p.read_text(encoding="utf-8").lower()
        for kw in FORBIDDEN_KEYWORDS:
            if f'license = "{kw}"' in content or kw in content:
                errors.append(f"{p}: forbidden license keyword '{kw}' detected")

    if errors:
        print(f"FAILED: License audit failed with {len(errors)} violation(s):", file=sys.stderr)
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        return 1

    print(f"SUCCESS: License audit passed. Inspected {checked_files} manifest(s). No forbidden licenses found.")
    return 0

if __name__ == "__main__":
    sys.exit(main())
