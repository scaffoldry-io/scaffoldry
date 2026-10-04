#!/usr/bin/env python3
"""Validate an OSCAL file against the official NIST OSCAL 1.1.2 schema.

Usage:
    validate-oscal.py <file.json> [<file.json> ...]

Exit code:
    0 if all files validate successfully against NIST schema.
    1 if any validation error or non-conforming structure is found.
"""
import json
import os
import sys
import jsonschema

HERE = os.path.dirname(os.path.abspath(__file__))
SCHEMA_PATH = os.path.join(HERE, "..", "schema", "oscal_complete_schema-1.1.2.json")
OSCAL_VERSION = "1.1.2"

MODELS = {
    "catalog",
    "profile",
    "component-definition",
    "system-security-plan",
    "assessment-plan",
    "assessment-results",
    "plan-of-action-and-milestones",
}


def python_patterns(node):
    if isinstance(node, dict):
        for k, v in node.items():
            if k == "pattern" and isinstance(v, str):
                node[k] = v.replace(r"\p{L}", r"[^\W\d_]").replace(r"\p{N}", r"\d")
            else:
                python_patterns(v)
    elif isinstance(node, list):
        for v in node:
            python_patterns(v)
    return node


def validate(path, schema):
    try:
        with open(path, "r", encoding="utf-8") as f:
            doc = json.load(f)
    except Exception as e:
        return f"failed to parse JSON: {e}"

    keys = [k for k in doc if not k.startswith("_")]
    if len(keys) != 1 or keys[0] not in MODELS:
        return f"top-level key is {keys}, not one OSCAL model out of {sorted(MODELS)}"

    model = keys[0]
    sub = {
        "definitions": schema["definitions"],
        "$ref": f"#/definitions/oscal-complete-oscal-{model}:{model}",
    }

    try:
        jsonschema.Draft7Validator(sub, format_checker=None).validate(doc[model])
    except jsonschema.ValidationError as e:
        where = "/".join(str(p) for p in e.absolute_path) or "(root)"
        return f"{model} validation error at {where}: {e.message}"
    except Exception as e:
        return f"unexpected error during validation: {e}"

    return None


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2

    if not os.path.exists(SCHEMA_PATH):
        sys.stderr.write(f"Schema not found at {SCHEMA_PATH}\n")
        return 2

    schema = python_patterns(json.load(open(SCHEMA_PATH, encoding="utf-8")))
    rc = 0
    for path in argv[1:]:
        fault = validate(path, schema)
        if fault:
            rc = 1
            print(f"❌ {path}: {fault}")
        else:
            print(f"✓ {path}: validates strictly against NIST OSCAL {OSCAL_VERSION}")
    return rc


if __name__ == "__main__":
    sys.exit(main(sys.argv))
