import json
import os
import unittest

import jsonschema
from referencing import Registry, Resource

HERE = os.path.dirname(os.path.abspath(__file__))
SCHEMAS = os.path.join(HERE, "..", "schema")


def load(name):
    with open(os.path.join(SCHEMAS, name), "r", encoding="utf-8") as f:
        return json.load(f)


SCHEMA = load("policy-library.schema.json")
APPROVER = load("approver-spec.schema.json")
REGISTRY = Registry().with_resource("approver-spec.schema.json", Resource.from_contents(APPROVER))
UUID = "00000000-0000-0000-0000-0000000000aa"
PASSAGE = "Student records are kept for seven years after the student leaves."


def check(name, doc):
    ref = {"$ref": f"policy-library.schema.json#/$defs/{name}"}
    registry = REGISTRY.with_resource("policy-library.schema.json", Resource.from_contents(SCHEMA))
    return list(jsonschema.Draft202012Validator(ref, registry=registry).iter_errors(doc))


POLICY = {"key": "records", "title": "Records retention", "organization_id": UUID, "owner": {"Person": {"eppn": "a@state.edu"}}, "reason": "Owned by the registrar"}
RULE = {"key": "retention", "statement": "Student records are kept for seven years.", "citations": [{"snapshot_id": UUID, "passage": PASSAGE}], "reason": "Read from the schedule"}


class PolicyLibrarySchema(unittest.TestCase):
    def test_the_schema_is_a_json_schema(self):
        jsonschema.Draft202012Validator.check_schema(SCHEMA)

    def test_good_bodies_pass(self):
        self.assertEqual(check("policy_create", POLICY), [])
        self.assertEqual(check("policy_patch", {"title": "New", "reason": "Renamed"}), [])
        self.assertEqual(check("source", {"title": "Schedule", "kind": "url", "href": "https://registrar.example.edu/schedule", "reason": "r"}), [])
        self.assertEqual(check("source", {"title": "Schedule", "kind": "text", "reason": "r"}), [])
        self.assertEqual(check("snapshot", {"text": PASSAGE, "reason": "r"}), [])
        self.assertEqual(check("rule", RULE), [])

    def test_what_the_server_refuses_the_schema_refuses_too(self):
        bad = [
            ("policy_create", {**POLICY, "key": "Records Policy"}),
            ("policy_create", {k: v for k, v in POLICY.items() if k != "reason"}),
            ("policy_create", {**POLICY, "owner": {"Group": "x"}}),
            ("policy_create", {**POLICY, "review_every_days": 0}),
            ("policy_patch", {"reason": "nothing to change"}),
            ("policy_patch", {"status": "Deleted", "reason": "r"}),
            ("source", {"title": "Page", "kind": "url", "href": "http://registrar.example.edu", "reason": "r"}),
            ("source", {"title": "Page", "kind": "url", "reason": "r"}),
            ("source", {"title": "Paste", "kind": "text", "href": "https://x.example.edu", "reason": "r"}),
            ("snapshot", {"text": "", "reason": "r"}),
            ("rule", {**RULE, "citations": []}),
            ("rule", {**RULE, "statement": "x" * 501}),
            ("rule", {**RULE, "citations": [{"snapshot_id": UUID, "passage": "too short"}]}),
        ]
        for name, doc in bad:
            self.assertNotEqual(check(name, doc), [], f"{name} should fail: {doc}")


if __name__ == "__main__":
    unittest.main()
