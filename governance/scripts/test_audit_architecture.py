#!/usr/bin/env python3
"""Tests for audit-architecture.py, the one-door rule. Run with: python3 -m unittest discover -s governance/scripts -p "test_*.py"."""

import importlib.util
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("audit_architecture", Path(__file__).with_name("audit-architecture.py"))
audit = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(audit)


def audit_file(rel, text):
    """Runs the one-door audit on a temp repository holding one Rust file."""
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        path = root / rel
        path.parent.mkdir(parents=True)
        path.write_text(text)
        return audit.audit_data_door(root)


SELECT = 'let sql = "SELECT * FROM dataset_records";\n'


class DataDoorTests(unittest.TestCase):
    def test_a_select_on_dataset_records_outside_the_data_crate_fails(self):
        errors = audit_file("crates/scaffoldry-server/src/new_reader.rs", SELECT)
        self.assertEqual(len(errors), 1)
        self.assertIn("new_reader.rs", errors[0])
        self.assertIn("dataset_records", errors[0])

    def test_each_guarded_table_fails(self):
        for table in ("record_index", "data_labels", "record_keys"):
            text = f'let sql = "DELETE FROM {table} WHERE id = 1";\n'
            self.assertEqual(len(audit_file("crates/scaffoldry-core/src/x.rs", text)), 1, table)

    def test_sql_split_across_lines_fails(self):
        text = 'let sql = "SELECT id FROM \\\n    dataset_records";\n'
        self.assertEqual(len(audit_file("crates/scaffoldry-server/src/x.rs", text)), 1)

    def test_the_data_crate_may_read_the_tables(self):
        self.assertEqual(audit_file("crates/scaffoldry-data/src/records.rs", SELECT), [])

    def test_a_comment_that_names_a_table_is_not_sql(self):
        self.assertEqual(audit_file("crates/scaffoldry-server/src/x.rs", "// reads FROM dataset_records, see the data crate\n"), [])

    def test_an_integration_test_may_seed_and_inspect_rows(self):
        self.assertEqual(audit_file("crates/scaffoldry-server/tests/some_test.rs", SELECT), [])

    def test_an_allow_listed_file_passes(self):
        rel = sorted(audit.DATA_DOOR_ALLOWED)[0]
        self.assertEqual(audit_file(rel, SELECT), [])

    def test_every_allow_list_entry_has_a_reason_and_a_move_note(self):
        for rel, (reason, move) in audit.DATA_DOOR_ALLOWED.items():
            self.assertTrue(reason.strip() and move.strip(), rel)

    def test_the_real_repository_passes(self):
        root = Path(__file__).resolve().parents[2]
        self.assertEqual(audit.audit_data_door(root), [])

    def test_no_allow_list_entry_is_stale(self):
        root = Path(__file__).resolve().parents[2]
        self.assertEqual(audit.stale_data_door_entries(root), [])


if __name__ == "__main__":
    unittest.main()
