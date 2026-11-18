#!/usr/bin/env python3
"""Slim the NIST OSCAL catalogs and baseline profiles.

Reads governance/catalog/source/ (fetched pinned, see governance/catalog/PROVENANCE.json).
Writes governance/catalog/sp800-53r5.slim.json, sp800-171r3.slim.json and baselines.json.
Standard library only. Each source file's SHA-256 is checked against PROVENANCE.json first.

Usage: slim-catalog.py
"""
import hashlib
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
CATALOG_DIR = os.path.join(HERE, "..", "catalog")
SOURCE_DIR = os.path.join(CATALOG_DIR, "source")

CATALOGS = [
    ("NIST_SP-800-53_rev5_catalog.json", "sp800-53r5", "sp800-53r5.slim.json"),
    ("NIST_SP800-171_rev3_catalog.json", "sp800-171r3", "sp800-171r3.slim.json"),
]
PROFILES = [
    ("low", "NIST_SP-800-53_rev5_LOW-baseline_profile.json"),
    ("moderate", "NIST_SP-800-53_rev5_MODERATE-baseline_profile.json"),
    ("high", "NIST_SP-800-53_rev5_HIGH-baseline_profile.json"),
]
INSERT = re.compile(r"\{\{\s*insert:\s*param,\s*([^\s}]+)\s*\}\}")


def fail(message):
    sys.exit(f"slim-catalog: {message}")


def verify_hash(data, expected, name):
    actual = hashlib.sha256(data).hexdigest()
    if actual != expected:
        fail(f"{name} does not match PROVENANCE.json (expected {expected}, got {actual})")


def read_source(name, provenance):
    path = os.path.join(SOURCE_DIR, name)
    if not os.path.isfile(path):
        fail(f"{path} is missing. Fetch the pinned files listed in PROVENANCE.json")
    entry = next((f for f in provenance["files"] if f["name"] == name), None)
    if entry is None:
        fail(f"{name} is not listed in PROVENANCE.json")
    with open(path, "rb") as f:
        data = f.read()
    verify_hash(data, entry["sha256"], name)
    return json.loads(data)


def is_withdrawn(node):
    return any(p.get("name") == "status" and p.get("value") == "withdrawn" for p in node.get("props", []))


def collect_params(node, found):
    """Every param in the catalog, by id. An insert may name a param on another control."""
    for param in node.get("params", []):
        found[param["id"]] = param
    for child in node.get("groups", []) + node.get("controls", []):
        collect_params(child, found)
    return found


def item_label(value):
    """OSCAL 800-53 labels are short ('a.', '1.'). 800-171 labels are long ('SR-03.01.01.c.02')."""
    if value.startswith("SR-"):
        return value.rsplit(".", 1)[-1] + "."
    return value


def flatten(part, params, depth=0):
    lines = []
    prose = part.get("prose")
    if prose:
        label = next((item_label(p["value"]) for p in part.get("props", []) if p.get("name") == "label"), "")
        text = INSERT.sub(lambda m: "[" + (params.get(m.group(1), {}).get("label") or m.group(1)) + "]", prose)
        lines.append("  " * depth + (label + " " if label else "") + text)
    for sub in part.get("parts", []):
        lines.extend(flatten(sub, params, depth + 1 if prose else depth))
    return lines


def statement(control, params):
    lines = []
    for part in control.get("parts", []):
        if part.get("name") == "statement":
            lines.extend(flatten(part, params))
    return "\n".join(lines)


def slim_catalog(doc, name):
    cat = doc["catalog"]
    params = collect_params(cat, {})
    out = []

    def visit(control, group, parent_id):
        if not is_withdrawn(control):
            row = {
                "id": control["id"],
                "title": control["title"],
                "family_id": group["id"],
                "family_title": group["title"],
            }
            if parent_id:
                row["parent_id"] = parent_id
            row["statement"] = statement(control, params)
            out.append(row)
        for child in control.get("controls", []):
            visit(child, group, control["id"])

    for group in cat["groups"]:
        for control in group.get("controls", []):
            visit(control, group, None)
    return {"catalog": name, "version": cat["metadata"]["version"], "controls": out}


def baseline_ids(profile, known):
    """A profile imports one catalog and lists the selected ids in include-controls[].with-ids."""
    ids = []
    for imported in profile["profile"]["imports"]:
        for include in imported.get("include-controls", []):
            ids.extend(include["with-ids"])
    missing = [i for i in ids if i not in known]
    if missing:
        fail(f"baseline names controls that are not in the slim catalog: {missing[:5]}")
    return ids


def write(name, doc):
    path = os.path.join(CATALOG_DIR, name)
    with open(path, "w", encoding="utf-8") as f:
        json.dump(doc, f, ensure_ascii=False, separators=(",", ":"))
        f.write("\n")
    return os.path.getsize(path)


def main():
    with open(os.path.join(CATALOG_DIR, "PROVENANCE.json"), encoding="utf-8") as f:
        provenance = json.load(f)
    slim_53 = None
    for source, name, target in CATALOGS:
        slim = slim_catalog(read_source(source, provenance), name)
        size = write(target, slim)
        print(f"{target}: {len(slim['controls'])} controls, {size} bytes")
        if name == "sp800-53r5":
            slim_53 = slim
    known = {c["id"] for c in slim_53["controls"]}
    baselines = {}
    version = None
    for name, source in PROFILES:
        profile = read_source(source, provenance)
        baselines[name] = baseline_ids(profile, known)
        version = profile["profile"]["metadata"]["version"]
        print(f"baseline {name}: {len(baselines[name])} controls")
    size = write("baselines.json", {"catalog": "sp800-53r5", "version": version, "baselines": baselines})
    print(f"baselines.json: {size} bytes")


if __name__ == "__main__":
    main()
