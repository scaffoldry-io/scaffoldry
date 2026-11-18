import importlib.util
import json
import os
import unittest

import jsonschema

HERE = os.path.dirname(os.path.abspath(__file__))
CATALOG = os.path.join(HERE, "..", "catalog")
SCHEMA = json.load(open(os.path.join(HERE, "..", "schema", "catalog-slim.schema.json"), encoding="utf-8"))

spec = importlib.util.spec_from_file_location("slim_catalog", os.path.join(HERE, "slim-catalog.py"))
slim = importlib.util.module_from_spec(spec)
spec.loader.exec_module(slim)


def catalog(groups, params=None):
    return {"catalog": {"metadata": {"version": "9.9"}, "groups": groups}}


def prop(name, value):
    return {"name": name, "value": value}


class Slimming(unittest.TestCase):
    def test_withdrawn_controls_are_skipped(self):
        doc = catalog([{"id": "ac", "title": "Access Control", "controls": [
            {"id": "ac-1", "title": "Policy", "parts": [{"name": "statement", "prose": "Do it."}]},
            {"id": "ac-9", "title": "Gone", "props": [prop("status", "withdrawn")],
             "parts": [{"name": "statement", "prose": "Old."}]},
        ]}])
        ids = [c["id"] for c in slim.slim_catalog(doc, "sp800-53r5")["controls"]]
        self.assertEqual(ids, ["ac-1"])

    def test_enhancements_carry_their_parent(self):
        doc = catalog([{"id": "ac", "title": "Access Control", "controls": [
            {"id": "ac-2", "title": "Account Management", "parts": [{"name": "statement", "prose": "Manage."}],
             "controls": [{"id": "ac-2.1", "title": "Automated", "parts": [{"name": "statement", "prose": "Automate."}]}]},
        ]}])
        rows = {c["id"]: c for c in slim.slim_catalog(doc, "sp800-53r5")["controls"]}
        self.assertNotIn("parent_id", rows["ac-2"])
        self.assertEqual(rows["ac-2.1"]["parent_id"], "ac-2")
        self.assertEqual(rows["ac-2.1"]["family_id"], "ac")
        self.assertEqual(rows["ac-2.1"]["family_title"], "Access Control")

    def test_params_become_labels_or_ids(self):
        doc = catalog([{"id": "ac", "title": "Access Control", "controls": [
            {"id": "ac-1", "title": "Policy",
             "params": [{"id": "p1", "label": "frequency"}, {"id": "p2", "select": {"choice": ["a", "b"]}}],
             "parts": [{"name": "statement", "parts": [
                 {"name": "item", "props": [prop("label", "a.")], "prose": "Review {{ insert: param, p1 }} and {{ insert: param, p2 }}."},
                 {"name": "item", "props": [prop("label", "b.")], "prose": "Second.",
                  "parts": [{"name": "item", "props": [prop("label", "1.")], "prose": "Nested."}]},
             ]}]},
        ]}])
        statement = slim.slim_catalog(doc, "sp800-53r5")["controls"][0]["statement"]
        self.assertNotIn("{{", statement)
        self.assertIn("Review [frequency] and [p2].", statement)
        self.assertEqual(statement.splitlines()[0], "a. Review [frequency] and [p2].")
        self.assertIn("1. Nested.", statement)

    def test_a_requirement_label_is_shortened(self):
        self.assertEqual(slim.item_label("SR-03.01.01.c.02"), "02.")
        self.assertEqual(slim.item_label("a."), "a.")

    def test_baseline_ids_missing_from_the_catalog_stop_the_run(self):
        profile = {"profile": {"imports": [{"include-controls": [{"with-ids": ["ac-1", "zz-99"]}]}]}}
        with self.assertRaises(SystemExit):
            slim.baseline_ids(profile, {"ac-1"})

    def test_a_changed_source_stops_the_run(self):
        with self.assertRaises(SystemExit):
            slim.verify_hash(b"abc", "0" * 64, "x.json")


class CommittedFiles(unittest.TestCase):
    def load(self, name):
        with open(os.path.join(CATALOG, name), encoding="utf-8") as f:
            return json.load(f)

    def test_schema_is_a_json_schema(self):
        jsonschema.Draft202012Validator.check_schema(SCHEMA)

    def test_the_schema_validates_all_three_slim_files(self):
        for name in ("sp800-53r5.slim.json", "sp800-171r3.slim.json", "baselines.json"):
            jsonschema.Draft202012Validator(SCHEMA).validate(self.load(name))

    def test_the_schema_refuses_a_bad_catalog(self):
        v = jsonschema.Draft202012Validator(SCHEMA)
        bad = {"catalog": "sp800-171r2", "version": "1", "controls": []}
        self.assertFalse(v.is_valid(bad))
        bad = {"catalog": "sp800-53r5", "version": "1", "controls": [{"id": "ac-1", "title": "T"}]}
        self.assertFalse(v.is_valid(bad))

    def test_no_statement_has_a_template_marker(self):
        for name in ("sp800-53r5.slim.json", "sp800-171r3.slim.json"):
            for c in self.load(name)["controls"]:
                self.assertNotIn("{{", c["statement"], c["id"])


if __name__ == "__main__":
    unittest.main()
