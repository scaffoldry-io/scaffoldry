import json
import os
import unittest

import jsonschema

HERE = os.path.dirname(os.path.abspath(__file__))
PLAN = os.path.join(HERE, "..", "schema", "label-change-plan.schema.json")
SETTINGS = os.path.join(HERE, "..", "schema", "sensitivity-settings.schema.json")


def load(path):
    with open(path, "r", encoding="utf-8") as f:
        return json.load(f)


def errors(path, doc):
    return list(jsonschema.Draft202012Validator(load(path)).iter_errors(doc))


GOOD = {
    "effective_at": "2026-12-30T09:00:00Z",
    "notice_days": 33,
    "notice_days_min": 30,
    "change": {"kind": "tags", "adds": ["phi"], "removes": []},
    "audiences": [
        {"id": "lose_access", "name": "People who lose access", "count": 5, "told": "Access to this dataset changes on the date. Ask for an exception if you need to keep it."},
        {"id": "stewards", "name": "Stewards", "count": 2, "told": "A change you decided takes effect on the date."},
    ],
    "timeline": [{"action": "first_notice", "days_before": 33}, {"action": "effective", "days_before": 0}],
    "channels": ["inbox", "email", "api"],
}


class LabelChangePlanSchema(unittest.TestCase):
    def test_the_schemas_are_json_schemas(self):
        jsonschema.Draft202012Validator.check_schema(load(PLAN))
        jsonschema.Draft202012Validator.check_schema(load(SETTINGS))

    def test_a_good_plan_passes(self):
        self.assertEqual(errors(PLAN, GOOD), [])

    def test_a_plan_with_a_person_or_an_unknown_part_is_refused(self):
        self.assertNotEqual(errors(PLAN, {**GOOD, "people": ["a@state.edu"]}), [])
        bad = json.loads(json.dumps(GOOD))
        bad["audiences"][0]["people"] = ["a@state.edu"]
        self.assertNotEqual(errors(PLAN, bad), [])
        bad = json.loads(json.dumps(GOOD))
        bad["audiences"][0]["id"] = "everyone"
        self.assertNotEqual(errors(PLAN, bad), [])

    def test_the_change_names_categories_and_nothing_else(self):
        for change in [{"kind": "rows", "adds": [], "removes": []}, {"kind": "tags", "adds": ["Zed Secretname"], "removes": []}, {"kind": "tags", "adds": [], "removes": [], "records": ["r1"]}]:
            self.assertNotEqual(errors(PLAN, {**GOOD, "change": change}), [], change)

    def test_a_plan_needs_every_part(self):
        for part in GOOD:
            doc = {k: v for k, v in GOOD.items() if k != part}
            self.assertNotEqual(errors(PLAN, doc), [], part)

    def test_the_notice_setting_takes_classes_of_one_to_365_days(self):
        self.assertEqual(errors(SETTINGS, {"protected": {"raise": 45, "lower": 21}, "other": {"raise": 14}}), [])
        self.assertEqual(errors(SETTINGS, {}), [])
        self.assertNotEqual(errors(SETTINGS, {"protected": {"raise": 0}}), [])
        self.assertNotEqual(errors(SETTINGS, {"protected": {"raise": 366}}), [])
        self.assertNotEqual(errors(SETTINGS, {"restricted": {"raise": 30}}), [])
        self.assertNotEqual(errors(SETTINGS, {"protected": {"sooner": 3}}), [])


if __name__ == "__main__":
    unittest.main()
