#!/usr/bin/env python3
"""Tests for check-migrations.py. Run with: python3 -m unittest discover -s governance/scripts -p "test_*.py"."""

import importlib.util
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("check_migrations", Path(__file__).with_name("check-migrations.py"))
cm = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cm)


class MigrationCheckTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.root = Path(self.dir.name)
        (self.root / cm.MIGRATIONS).mkdir(parents=True)
        (self.root / cm.REGISTRY).parent.mkdir(parents=True)

    def tearDown(self):
        self.dir.cleanup()

    def world(self, files, registered):
        for f in files:
            (self.root / cm.MIGRATIONS / f).write_text("SELECT 1;\n")
        lines = "\n".join(f'    ("{r}", SCHEMA),' for r in registered)
        (self.root / cm.REGISTRY).write_text(f"let migrations = [\n{lines}\n];\n")
        return cm.problems(self.root)

    def test_a_clean_set_passes(self):
        self.assertEqual(self.world(["0001_a.sql", "0002_b.sql"], ["0001_a.sql", "0002_b.sql"]), [])

    def test_a_shared_number_fails_and_names_both_files(self):
        found = self.world(["0054_imports.sql", "0054_people_indexes.sql"], ["0054_imports.sql", "0054_people_indexes.sql"])
        self.assertEqual(len(found), 1)
        self.assertIn("0054_imports.sql", found[0])
        self.assertIn("0054_people_indexes.sql", found[0])

    def test_an_unregistered_file_fails(self):
        found = self.world(["0001_a.sql", "0002_b.sql"], ["0001_a.sql"])
        self.assertTrue(any("0002_b.sql" in p and "never runs" in p for p in found))

    def test_a_registered_name_with_no_file_fails(self):
        found = self.world(["0001_a.sql"], ["0001_a.sql", "0002_gone.sql"])
        self.assertTrue(any("0002_gone.sql" in p for p in found))

    def test_a_badly_named_file_fails(self):
        found = self.world(["54_imports.sql"], [])
        self.assertTrue(any("54_imports.sql" in p for p in found))

    def test_gaps_in_numbering_are_allowed(self):
        self.assertEqual(self.world(["0001_a.sql", "0007_b.sql"], ["0001_a.sql", "0007_b.sql"]), [])


class RealRepositoryTests(unittest.TestCase):
    def test_the_repository_passes(self):
        root = Path(__file__).resolve().parents[2]
        self.assertEqual(cm.problems(root), [])


if __name__ == "__main__":
    unittest.main()
