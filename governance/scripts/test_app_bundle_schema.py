"""Tests for governance/schema/app-bundle.schema.json. Run with: python3 -m unittest discover -s governance/scripts -p "test_*.py"."""

import hashlib
import json
import os
import unittest

import jsonschema
from referencing import Registry, Resource

HERE = os.path.dirname(os.path.abspath(__file__))


def load(*parts):
    with open(os.path.join(HERE, "..", *parts), "r", encoding="utf-8") as f:
        return json.load(f)


BUNDLE_SCHEMA = load("schema", "app-bundle.schema.json")
MANIFEST_SCHEMA = load("schema", "app-manifest.schema.json")
# The bundle's manifest is the app manifest, so the schema refers to that one.
REGISTRY = Registry().with_resources([(MANIFEST_SCHEMA["$id"], Resource.from_contents(MANIFEST_SCHEMA))])
VALIDATOR = jsonschema.Draft202012Validator(BUNDLE_SCHEMA, registry=REGISTRY)
EXAMPLE = load("fixtures", "app-bundle.example.json")


def sealed(doc):
    """The hash a bundle carries: SHA-256 of the document without its own sha256, as compact JSON with sorted keys."""
    body = {k: v for k, v in doc.items() if k != "sha256"}
    text = json.dumps(body, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def changed(**edits):
    doc = json.loads(json.dumps(EXAMPLE))
    for key, value in edits.items():
        doc[key] = value
    return doc


class AppBundleSchema(unittest.TestCase):
    def test_the_schema_is_a_json_schema(self):
        jsonschema.Draft202012Validator.check_schema(BUNDLE_SCHEMA)

    def test_the_document_the_server_exports_passes(self):
        # The example is the export of a real app. A Rust test fails when the export stops matching it.
        self.assertEqual(list(VALIDATOR.iter_errors(EXAMPLE)), [])

    def test_the_hash_can_be_worked_out_without_the_server(self):
        self.assertEqual(sealed(EXAMPLE), EXAMPLE["sha256"])

    def test_a_bundle_without_sample_data_or_webhooks_passes(self):
        doc = changed()
        doc.pop("sample_data")
        doc["integrations"] = {"webhooks": []}
        self.assertEqual(list(VALIDATOR.iter_errors(doc)), [])

    def test_a_webhook_comes_in_off_and_names_its_secret(self):
        hook = {"name": "notify", "url": "https://hooks.example.edu/notify", "events": ["record.created"], "enabled": False, "secret_name": "notify-secret"}
        self.assertEqual(list(VALIDATOR.iter_errors(changed(integrations={"webhooks": [hook]}))), [])

    def test_what_the_server_refuses_the_schema_refuses_too(self):
        hook = {"name": "notify", "url": "https://hooks.example.edu/notify", "enabled": False}
        bad = {
            "a secret value in a webhook": changed(integrations={"webhooks": [{**hook, "secret": "s3cr3t"}]}),
            "a webhook that is not https": changed(integrations={"webhooks": [{**hook, "url": "http://hooks.example.edu/notify"}]}),
            "a part nobody knows": changed(members=[{"eppn": "someone@state.edu"}]),
            "a comments part": changed(comments=[]),
            "a version this server does not read": changed(bundle_version=2),
            "a hash that is not a hash": changed(sha256="ABC"),
            "no hash": {k: v for k, v in EXAMPLE.items() if k != "sha256"},
            "a page source filed under something that is not a hash": changed(page_sources={"board": "export default function Page() {}"}),
            "a personal view": changed(views=[{"name": "Mine", "kind": "grid", "table_id": "cases", "ownership": "personal", "config": {}}]),
            "more than 5,000 sample records in a table": changed(sample_data={"cases": [{"title": "x"}] * 5001}),
            "a table of sample data that is not a list": changed(sample_data={"cases": {"title": "x"}}),
        }
        for what, doc in bad.items():
            with self.subTest(what):
                self.assertNotEqual(list(VALIDATOR.iter_errors(doc)), [], f"the schema accepts {what}")


if __name__ == "__main__":
    unittest.main()
