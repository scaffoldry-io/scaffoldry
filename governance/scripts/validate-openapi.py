#!/usr/bin/env python3
"""Check the generated OpenAPI description of the fixture app.

Usage:
    validate-openapi.py [fixtures_dir]

The directory holds `fixture.openapi.json`, the document the server builds for the fixture app (a Rust test keeps it
the same as what the server builds), and `examples.json`, with records that must pass and records that must fail
against each table's component schema.

Exit code 0 when the document and every example behave, 1 otherwise.
"""
import json
import os
import sys

import jsonschema

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_FIXTURES = os.path.join(HERE, "..", "fixtures", "openapi")
HTTP_METHODS = ("get", "put", "post", "delete", "options", "head", "patch", "trace")


def load(path):
    with open(path, "r", encoding="utf-8") as f:
        return json.load(f)


def refs_of(node):
    """Every $ref string under node."""
    if isinstance(node, dict):
        for key, value in node.items():
            if key == "$ref" and isinstance(value, str):
                yield value
            else:
                yield from refs_of(value)
    elif isinstance(node, list):
        for item in node:
            yield from refs_of(item)


def resolve(doc, ref):
    node = doc
    for part in ref.removeprefix("#/").split("/"):
        if not isinstance(node, dict) or part not in node:
            return None
        node = node[part]
    return node


def structure_problems(doc):
    """What is missing from the document, as sentences."""
    problems = []
    if not str(doc.get("openapi", "")).startswith("3.1"):
        problems.append("`openapi` must be a 3.1 version")
    info = doc.get("info") or {}
    for key in ("title", "version"):
        if not info.get(key):
            problems.append(f"`info.{key}` is missing")
    paths = doc.get("paths")
    if not isinstance(paths, dict) or not paths:
        problems.append("`paths` is missing or empty")
        paths = {}
    seen = set()
    for path, item in paths.items():
        if not path.startswith("/"):
            problems.append(f"the path {path!r} does not start with /")
        for method, op in item.items():
            if method not in HTTP_METHODS:
                continue
            where = f"{method.upper()} {path}"
            if not op.get("responses"):
                problems.append(f"{where} has no responses")
            op_id = op.get("operationId")
            if not op_id:
                problems.append(f"{where} has no operationId")
            elif op_id in seen:
                problems.append(f"{where} repeats the operationId {op_id!r}")
            seen.add(op_id)
    components = doc.get("components") or {}
    if not isinstance(components.get("schemas"), dict) or not components["schemas"]:
        problems.append("`components.schemas` is missing or empty")
    scheme = (components.get("securitySchemes") or {}).get("bearerAuth") or {}
    if scheme.get("type") != "http" or scheme.get("scheme") != "bearer":
        problems.append("`components.securitySchemes.bearerAuth` must be an http bearer scheme")
    if not doc.get("security"):
        problems.append("`security` is missing")
    for ref in set(refs_of(doc)):
        if resolve(doc, ref) is None:
            problems.append(f"the reference {ref} does not resolve")
    return problems


def example_problems(doc, examples):
    problems = []
    schemas = (doc.get("components") or {}).get("schemas") or {}
    for kind, expect_valid in (("valid", True), ("invalid", False)):
        for table, records in (examples.get(kind) or {}).items():
            schema = schemas.get(table)
            if schema is None:
                problems.append(f"the examples name the table {table!r}, which has no schema")
                continue
            try:
                jsonschema.Draft202012Validator.check_schema(schema)
            except jsonschema.SchemaError as e:
                problems.append(f"the schema of {table!r} is not a JSON Schema: {e.message}")
                continue
            # The schema can point at others in the same document, so validate against the whole of it.
            wrapper = {"$ref": f"#/components/schemas/{table}", "components": doc["components"]}
            validator = jsonschema.Draft202012Validator(wrapper)
            for i, record in enumerate(records):
                errors = [e.message for e in validator.iter_errors(record)]
                if expect_valid and errors:
                    problems.append(f"{kind} example {i} of {table!r} fails: {errors[0]}")
                if not expect_valid and not errors:
                    problems.append(f"{kind} example {i} of {table!r} passes, and it must not")
    return problems


def main(argv):
    fixtures = argv[1] if len(argv) > 1 else DEFAULT_FIXTURES
    doc = load(os.path.join(fixtures, "fixture.openapi.json"))
    examples = load(os.path.join(fixtures, "examples.json"))
    problems = structure_problems(doc) + example_problems(doc, examples)
    for p in problems:
        print(f"FAIL: {p}")
    if problems:
        return 1
    count = sum(len(v) for kind in ("valid", "invalid") for v in examples.get(kind, {}).values())
    print(f"SUCCESS: the OpenAPI description is well formed and {count} example records behave.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
