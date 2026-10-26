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


SCHEMA = load("authority.schema.json")
REGISTRY = Registry().with_resource("approver-spec.schema.json", Resource.from_contents(load("approver-spec.schema.json")))
UUID = "00000000-0000-0000-0000-0000000000aa"
OFFICER = {"Position": {"key": "privacy-officer", "from": "Workspace", "walk_up": True}}


def check(name, doc):
    registry = REGISTRY.with_resource("authority.schema.json", Resource.from_contents(SCHEMA))
    validator = jsonschema.Draft202012Validator({"$ref": f"authority.schema.json#/$defs/{name}"}, registry=registry)
    return list(validator.iter_errors(doc))


class AuthoritySchema(unittest.TestCase):
    def test_the_schema_is_a_json_schema(self):
        jsonschema.Draft202012Validator.check_schema(SCHEMA)

    def test_good_documents_pass(self):
        self.assertEqual(check("signal", {"organization_id": UUID, "check_name": "sensitivity", "tier": "High"}), [])
        self.assertEqual(check("row", {"tier": "Moderate", "path": "second_person"}), [])
        self.assertEqual(check("row", {"tier": "High", "path": "approver", "approver": OFFICER, "control_id": "CM-03"}), [])
        self.assertEqual(check("change", {"kind": "signal", "check_name": "data_loss", "tier": "High"}), [])
        self.assertEqual(check("change", {"kind": "row", "row": {"tier": "High", "path": "approver", "approver": OFFICER}}), [])

    def test_what_the_server_refuses_the_schema_refuses_too(self):
        bad = [
            ("signal", {"organization_id": UUID, "check_name": "sensitivity", "tier": "Critical"}),
            ("signal", {"organization_id": UUID, "tier": "High"}),
            ("row", {"tier": "Moderate", "path": "approver"}),
            ("row", {"tier": "Moderate", "path": "direct", "approver": OFFICER}),
            ("row", {"tier": "Moderate", "path": "committee"}),
            ("row", {"tier": "Moderate", "path": "approver", "approver": {"Group": "x"}}),
            ("change", {"kind": "row", "row": {"tier": "Low"}}),
            ("change", {"kind": "unknown"}),
        ]
        for name, doc in bad:
            self.assertNotEqual(check(name, doc), [], f"{name} should fail: {doc}")


if __name__ == "__main__":
    unittest.main()
