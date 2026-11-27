import json
import os
import unittest

import jsonschema

HERE = os.path.dirname(os.path.abspath(__file__))
SETTINGS = os.path.join(HERE, "..", "schema", "sensitivity-settings.schema.json")
MANIFEST = os.path.join(HERE, "..", "schema", "app-manifest.schema.json")


def load(path):
    with open(path, "r", encoding="utf-8") as f:
        return json.load(f)


def settings_errors(doc):
    return list(jsonschema.Draft202012Validator(load(SETTINGS)).iter_errors(doc))


def audience_errors(audience):
    """Checks one table's audience against the table definition inside the manifest schema."""
    schema = load(MANIFEST)
    table = schema["$defs"]["table"] if "table" in schema.get("$defs", {}) else None
    if table is None:
        raise unittest.SkipTest("the manifest schema keeps its table definition elsewhere")
    validator = jsonschema.Draft202012Validator({**table, "$defs": schema["$defs"]})
    base = {"id": "t", "name": "T", "slug": "t", "fields": []}
    return list(validator.iter_errors({**base, "audience": audience}))


CONDITIONS = [{"id": "ferpa_training", "name": "FERPA training", "owner": "registrar"}]


class ConditionsSchema(unittest.TestCase):
    def test_a_closed_list_of_conditions_passes(self):
        self.assertEqual(settings_errors(CONDITIONS), [])

    def test_a_condition_has_an_id_a_name_and_an_owner_and_nothing_else(self):
        self.assertNotEqual(settings_errors([{"id": "ferpa_training", "name": "FERPA training"}]), [])
        self.assertNotEqual(settings_errors([{**CONDITIONS[0], "expression": "a and b"}]), [])
        self.assertNotEqual(settings_errors([{**CONDITIONS[0], "id": "Not An Id"}]), [])

    def test_a_category_names_conditions_by_id(self):
        category = {"id": "ferpa", "name": "FERPA", "protected": True, "detectors": [], "conditions": ["ferpa_training"]}
        self.assertEqual(settings_errors([category]), [])
        self.assertNotEqual(settings_errors([{**category, "conditions": ["Not An Id"]}]), [])
        self.assertNotEqual(settings_errors([{**category, "conditions": ["a"] * 2}]), [], "a condition is named once")


class AudienceSchema(unittest.TestCase):
    def test_a_policy_names_a_field_and_the_fields_it_reaches(self):
        self.assertEqual(audience_errors([{"field": "advisor", "grants": "write", "fields": ["name"], "unit_acceptance": True}]), [])
        self.assertEqual(audience_errors([{"field": "advisor", "fields": ["name"]}]), [], "read is the default")

    def test_a_policy_that_grants_nothing_or_grants_something_else_is_refused(self):
        self.assertNotEqual(audience_errors([{"field": "advisor", "fields": []}]), [])
        self.assertNotEqual(audience_errors([{"field": "advisor", "grants": "admin", "fields": ["name"]}]), [])
        self.assertNotEqual(audience_errors([{"field": "advisor", "fields": ["name"], "who": "everyone"}]), [])
        self.assertNotEqual(audience_errors([{"fields": ["name"]}]), [])


if __name__ == "__main__":
    unittest.main()
