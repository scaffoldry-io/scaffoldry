import json
import os
import unittest

import jsonschema

HERE = os.path.dirname(os.path.abspath(__file__))
SCHEMA_PATH = os.path.join(HERE, "..", "schema", "access-request.schema.json")


def load():
    with open(SCHEMA_PATH, "r", encoding="utf-8") as f:
        return json.load(f)


def check(name, doc):
    validator = jsonschema.Draft202012Validator({"$ref": f"#/$defs/{name}", "$defs": load()["$defs"]})
    return list(validator.iter_errors(doc))


DEFAULT_RULE = {"max_role": "viewer", "max_sensitivity": "Internal", "visibilities": ["departmental", "institutional"], "own_unit": True, "max_days": 30}


class AccessRequestSchema(unittest.TestCase):
    def test_the_schema_is_a_json_schema(self):
        jsonschema.Draft202012Validator.check_schema(load())

    def test_good_documents_pass(self):
        self.assertEqual(check("rule", DEFAULT_RULE), [])
        self.assertEqual(check("rule", {**DEFAULT_RULE, "affiliations": ["staff"]}), [])
        self.assertEqual(check("request", {"object_kind": "app", "object_id": "scholarships", "role": "viewer", "reason": "Audit", "days": 14}), [])
        self.assertEqual(check("request", {"object_kind": "workspace", "object_id": "ws-1", "role": "editor", "clearance": "Internal", "reason": "Work", "days": 30}), [])

    def test_what_the_server_refuses_the_schema_refuses_too(self):
        bad = [
            ("rule", {**DEFAULT_RULE, "visibilities": ["departmental", "restricted"]}),
            ("rule", {**DEFAULT_RULE, "visibilities": []}),
            ("rule", {**DEFAULT_RULE, "max_role": "emperor"}),
            ("rule", {**DEFAULT_RULE, "max_sensitivity": "Secret"}),
            ("rule", {**DEFAULT_RULE, "max_days": 0}),
            ("rule", {**DEFAULT_RULE, "max_days": 366}),
            ("rule", {**DEFAULT_RULE, "affiliations": []}),
            ("rule", {**DEFAULT_RULE, "unknown": True}),
            ("rule", {"max_role": "viewer"}),
            ("request", {"object_kind": "table", "object_id": "t", "role": "viewer", "reason": "r", "days": 7}),
            ("request", {"object_kind": "app", "object_id": "a", "role": "viewer", "reason": "", "days": 7}),
            ("request", {"object_kind": "app", "object_id": "a", "role": "viewer", "reason": "r", "days": 0}),
            ("request", {"object_kind": "app", "object_id": "a", "role": "viewer", "days": 7}),
        ]
        for name, doc in bad:
            self.assertNotEqual(check(name, doc), [], f"{name} should fail: {doc}")


if __name__ == "__main__":
    unittest.main()
