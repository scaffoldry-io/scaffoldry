"""The nextest config keeps its promise about tests that measure time.

A test that asserts a wall-clock limit has to run alone, or the tests beside it decide whether it passes. The config lists
those tests by name. A name that is missing, or misspelled, protects nothing and says nothing. These tests catch both.
"""

import re
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# Timing tests whose own file says they run alone.
MUST_RUN_ALONE = ["a_receipt_write_adds_under_a_millisecond_at_the_median"]


def run_alone_names() -> set[str]:
    """The test names in every override that takes all the test threads."""
    config = tomllib.loads((ROOT / ".config" / "nextest.toml").read_text())
    names: set[str] = set()
    for override in config["profile"]["default"]["overrides"]:
        if override.get("threads-required") == "num-test-threads":
            # A filter expression such as `test(=a_name) | test(=another_name)`.
            names |= set(re.findall(r"test\(=([A-Za-z0-9_]+)\)", override["filter"]))
    return names


def defined_functions() -> set[str]:
    found: set[str] = set()
    for path in (ROOT / "crates").rglob("*.rs"):
        found |= set(re.findall(r"\bfn\s+([A-Za-z0-9_]+)\s*[(<]", path.read_text(errors="replace")))
    return found


class NextestConfigTest(unittest.TestCase):
    def test_a_timing_test_that_says_it_runs_alone_is_on_the_list(self):
        listed = run_alone_names()
        for name in MUST_RUN_ALONE:
            self.assertIn(name, listed, f"{name} asserts a wall-clock limit but shares the machine with three other tests")

    def test_every_name_on_the_list_is_a_test_that_exists(self):
        missing = sorted(run_alone_names() - defined_functions())
        self.assertEqual(missing, [], f"on the run-alone list but not defined anywhere, so they protect nothing: {missing}")


if __name__ == "__main__":
    unittest.main()
