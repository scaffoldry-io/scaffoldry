#!/usr/bin/env python3
"""Tests for rust-test-groups.py. Run with: python3 -m unittest discover -s governance/scripts -p "test_*.py"."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("rust_test_groups", Path(__file__).with_name("rust-test-groups.py"))
rtg = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(rtg)

REPO = Path(__file__).resolve().parents[2]


def tree(files, heavy):
    root = Path(tempfile.mkdtemp())
    for f in files:
        p = root / "crates" / "a" / "tests" / f
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text("")
    (root / "governance").mkdir()
    (root / "governance" / "rust-heavy-tests.json").write_text(json.dumps({"heavy": heavy}))
    return root


class GroupTests(unittest.TestCase):
    def test_every_test_file_lands_in_exactly_one_group(self):
        root = tree(["a_test.rs", "b_test.rs", "c_test.rs"], ["b_test"])
        heavy, light = rtg.groups(root)
        self.assertEqual(heavy, ["b_test"])
        self.assertEqual(light, ["a_test", "c_test"])

    def test_a_heavy_name_with_no_file_is_an_error(self):
        root = tree(["a_test.rs"], ["gone_test"])
        with self.assertRaises(rtg.GroupError) as e:
            rtg.groups(root)
        self.assertIn("gone_test", str(e.exception))

    def test_a_file_name_used_in_two_crates_counts_once(self):
        root = tree(["a_test.rs"], [])
        other = root / "crates" / "b" / "tests"
        other.mkdir(parents=True)
        (other / "a_test.rs").write_text("")
        self.assertEqual(rtg.groups(root), ([], ["a_test"]))

    def test_cargo_args_light_adds_lib_and_bins_and_heavy_does_not(self):
        root = tree(["a_test.rs", "b_test.rs"], ["b_test"])
        self.assertEqual(rtg.cargo_args(root, "heavy"), ["--test", "b_test"])
        self.assertEqual(rtg.cargo_args(root, "light"), ["--lib", "--bins", "--test", "a_test"])

    def test_the_real_repository_lists_only_files_that_exist(self):
        heavy, light = rtg.groups(REPO)
        self.assertTrue(heavy)
        self.assertTrue(light)
        self.assertFalse(set(heavy) & set(light))


if __name__ == "__main__":
    unittest.main()
