import copy
import importlib.util
import json
import os
import shutil
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURES = os.path.join(HERE, "..", "fixtures", "openapi")

spec = importlib.util.spec_from_file_location("validate_openapi", os.path.join(HERE, "validate-openapi.py"))
validate_openapi = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validate_openapi)


def load(name):
    with open(os.path.join(FIXTURES, name), "r", encoding="utf-8") as f:
        return json.load(f)


class ValidateOpenApi(unittest.TestCase):
    def test_the_checked_in_document_passes(self):
        self.assertEqual(validate_openapi.main(["validate-openapi.py", FIXTURES]), 0)

    def test_a_document_without_the_required_keys_fails(self):
        doc = load("fixture.openapi.json")
        broken = copy.deepcopy(doc)
        del broken["openapi"]
        del broken["components"]["securitySchemes"]
        problems = validate_openapi.structure_problems(broken)
        self.assertTrue(any("`openapi`" in p for p in problems), problems)
        self.assertTrue(any("bearerAuth" in p for p in problems), problems)

    def test_a_reference_that_does_not_resolve_fails(self):
        broken = copy.deepcopy(load("fixture.openapi.json"))
        first = next(iter(broken["components"]["schemas"].values()))
        first["x-bad"] = {"$ref": "#/components/schemas/Missing"}
        self.assertTrue(any("does not resolve" in p for p in validate_openapi.structure_problems(broken)))

    def test_an_invalid_example_that_passes_and_a_valid_one_that_fails_are_reported(self):
        doc = load("fixture.openapi.json")
        examples = {"valid": {"cases": [{"status": "open"}]}, "invalid": {"cases": [{"title": "fine"}]}}
        problems = validate_openapi.example_problems(doc, examples)
        self.assertEqual(len(problems), 2, problems)

    def test_the_script_exits_one_on_a_broken_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            for name in ("fixture.openapi.json", "examples.json"):
                shutil.copy(os.path.join(FIXTURES, name), tmp)
            doc = load("fixture.openapi.json")
            doc["openapi"] = "2.0"
            with open(os.path.join(tmp, "fixture.openapi.json"), "w", encoding="utf-8") as f:
                json.dump(doc, f)
            self.assertEqual(validate_openapi.main(["validate-openapi.py", tmp]), 1)


if __name__ == "__main__":
    unittest.main()
