import json
import os
import unittest

import jsonschema

HERE = os.path.dirname(os.path.abspath(__file__))
with open(os.path.join(HERE, "..", "schema", "app-client.schema.json"), "r", encoding="utf-8") as f:
    SCHEMA = json.load(f)
VALIDATOR = jsonschema.Draft202012Validator(SCHEMA)

GOOD = {
    "name": "Grant tracker",
    "operator": "Physics Department",
    "kind": "public",
    "redirect_uris": ["https://tracker.example.edu/callback"],
    "ceiling": {"apps": [{"app_slug": "grants", "tables": [{"table_id": "proposals", "read": ["title", "status"], "write": ["status"], "create": True, "delete": False}]}]},
}


def with_change(**changes):
    doc = json.loads(json.dumps(GOOD))
    doc.update(changes)
    return doc


class AppClientSchema(unittest.TestCase):
    def test_the_schema_is_a_json_schema(self):
        jsonschema.Draft202012Validator.check_schema(SCHEMA)

    def test_a_registration_passes(self):
        self.assertEqual(list(VALIDATOR.iter_errors(GOOD)), [])

    def test_a_confidential_client_may_ask_for_a_service_identity(self):
        doc = with_change(kind="confidential", service_identity=True)
        self.assertEqual(list(VALIDATOR.iter_errors(doc)), [])

    def test_what_the_server_refuses_the_schema_refuses_too(self):
        bad = [
            with_change(kind="hybrid"),
            with_change(redirect_uris=[]),
            with_change(redirect_uris=["https://*.example.edu/callback"]),
            with_change(redirect_uris=["https://tracker.example.edu/callback?next=/"]),
            with_change(redirect_uris=["http://tracker.example.edu/callback"]),
            with_change(ceiling={"apps": []}),
            with_change(ceiling={"apps": [{"app_slug": "grants", "tables": []}]}),
            with_change(unknown="x"),
            with_change(service_identity=True),
        ]
        for doc in bad:
            self.assertTrue(list(VALIDATOR.iter_errors(doc)), doc)


if __name__ == "__main__":
    unittest.main()
