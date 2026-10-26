import json
import os
import unittest

import jsonschema

HERE = os.path.dirname(os.path.abspath(__file__))
SCHEMA_PATH = os.path.join(HERE, "..", "schema", "access-draft.schema.json")


def errors(doc):
    with open(SCHEMA_PATH, "r", encoding="utf-8") as f:
        schema = json.load(f)
    return list(jsonschema.Draft202012Validator(schema).iter_errors(doc))


GRANT = {"holder_kind": "person", "holder_id": "a@state.edu", "role": "viewer", "anchor_kind": "container", "anchor_id": "app:x"}


class AccessDraftSchema(unittest.TestCase):
    def test_good_drafts_pass(self):
        self.assertEqual(errors({"kind": "grant", "change": GRANT, "reason": "Audit"}), [])
        self.assertEqual(errors({"kind": "grant", "change": GRANT, "reason": "Audit", "preview": True}), [])
        self.assertEqual(errors({"kind": "revoke", "change": {"grant_id": 4}, "reason": "Done"}), [])
        self.assertEqual(errors({"kind": "hierarchy", "change": {"key": "supervision", "name": "Supervision", "kind": "people", "source": "scim"}, "reason": "r"}), [])
        self.assertEqual(errors({"kind": "request_rule", "change": {"organization_id": "00000000-0000-0000-0000-000000000001", "retire": "x"}, "reason": "r"}), [])

    def test_what_the_server_refuses_the_schema_refuses_too(self):
        bad = [
            {"kind": "mask", "change": {}, "reason": "r"},
            {"kind": "coordinates", "change": {}, "reason": "r"},
            {"kind": "grant", "change": {"holder_id": "a@state.edu"}, "reason": "r"},
            {"kind": "revoke", "change": {}, "reason": "r"},
            {"kind": "revoke", "change": {"grant_id": 0}, "reason": "r"},
            {"kind": "grant", "change": GRANT},
            {"kind": "grant", "change": GRANT, "reason": ""},
            {"kind": "grant", "change": GRANT, "reason": "r", "confirm": True},
            {"kind": "request_rule", "change": {"organization_id": "x"}, "reason": "r"},
        ]
        for doc in bad:
            self.assertNotEqual(errors(doc), [], f"should fail: {doc}")


if __name__ == "__main__":
    unittest.main()
