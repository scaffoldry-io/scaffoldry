#!/usr/bin/env python3
"""Check the app manifest fixtures against governance/schema/app-manifest.schema.json.

Usage:
    validate-manifest.py [fixtures_dir]

Every file under valid/ must pass. Every file under invalid/ must fail.
Exit code 0 when all fixtures behave, 1 otherwise.
"""
import glob
import json
import os
import sys

import jsonschema

HERE = os.path.dirname(os.path.abspath(__file__))
SCHEMA_PATH = os.path.join(HERE, "..", "schema", "app-manifest.schema.json")
DEFAULT_FIXTURES = os.path.join(HERE, "..", "..", "crates", "scaffoldry-engine", "tests", "fixtures", "manifests")


def errors_of(validator, path):
    with open(path, "r", encoding="utf-8") as f:
        doc = json.load(f)
    return [e.message for e in validator.iter_errors(doc)]


def main(argv):
    fixtures = argv[1] if len(argv) > 1 else DEFAULT_FIXTURES
    with open(SCHEMA_PATH, "r", encoding="utf-8") as f:
        schema = json.load(f)
    jsonschema.Draft202012Validator.check_schema(schema)
    validator = jsonschema.Draft202012Validator(schema)

    failed = 0
    checked = 0
    for kind in ("valid", "invalid"):
        for path in sorted(glob.glob(os.path.join(fixtures, kind, "*.json"))):
            checked += 1
            errs = errors_of(validator, path)
            name = os.path.relpath(path, fixtures)
            if kind == "valid" and errs:
                failed += 1
                print(f"FAIL {name}: should pass, got: {errs[0]}")
            elif kind == "invalid" and not errs:
                failed += 1
                print(f"FAIL {name}: should be refused and was accepted")
    if checked == 0:
        print(f"FAIL no fixtures found under {fixtures}")
        return 1
    if failed:
        return 1
    print(f"SUCCESS: {checked} manifest fixtures behave as the schema says.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
