import json
import os
import unittest

import jsonschema

HERE = os.path.dirname(os.path.abspath(__file__))
SCHEMA = os.path.join(HERE, "..", "schema", "row-share-assessment.schema.json")


def load():
    with open(SCHEMA, "r", encoding="utf-8") as f:
        return json.load(f)


def errors(doc):
    return list(jsonschema.Draft202012Validator(load()).iter_errors(doc))


GOOD = {
    "version": 1,
    "source": "server_precheck",
    "reason": {"fits": "not_assessed", "characters": 41, "short": False},
    "exposure": {
        "rows": 2, "rows_inside_default": 0, "rows_beyond_default": 1, "rows_restricted": 1,
        "fields": ["name", "score"], "access": "write", "categories": ["phi"], "protected_fields": [],
    },
    "duration": {"days": 60, "max_days": 30, "too_long": True},
    "unusual": ["write_access", "restricted_category", "longer_than_setting"],
    "narrative": None,
    "advisory": "These are the server's own checks, with no model. They are advice. A person decides.",
}


class RowShareAssessmentSchema(unittest.TestCase):
    def test_the_schema_is_a_json_schema(self):
        jsonschema.Draft202012Validator.check_schema(load())

    def test_a_good_assessment_passes(self):
        self.assertEqual(errors(GOOD), [])

    def test_a_narrative_slot_takes_an_author_and_text(self):
        doc = json.loads(json.dumps(GOOD))
        doc["narrative"] = {"by": "review.agent", "text": "The reason names the term and the course."}
        self.assertEqual(errors(doc), [])
        doc["narrative"] = {"text": "No author"}
        self.assertNotEqual(errors(doc), [])

    def test_a_value_or_a_row_id_has_no_place_to_go(self):
        self.assertNotEqual(errors({**GOOD, "records": ["r1"]}), [])
        doc = json.loads(json.dumps(GOOD))
        doc["exposure"]["values"] = ["Ada Lovelace"]
        self.assertNotEqual(errors(doc), [])
        doc = json.loads(json.dumps(GOOD))
        doc["unusual"] = ["a free-text remark"]
        self.assertNotEqual(errors(doc), [])

    def test_the_server_never_writes_a_verdict_it_cannot_reach(self):
        doc = json.loads(json.dumps(GOOD))
        doc["reason"]["fits"] = "approved"
        self.assertNotEqual(errors(doc), [])


if __name__ == "__main__":
    unittest.main()
