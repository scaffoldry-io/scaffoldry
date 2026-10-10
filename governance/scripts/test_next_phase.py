#!/usr/bin/env python3
"""Tests for next-phase.py. Run with: python3 -m unittest discover -s governance/scripts -p "test_*.py"."""

import importlib.util
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("next_phase", Path(__file__).with_name("next-phase.py"))
np = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(np)

README = """# Workplan

| Order | Brief | Phase | What is true after it | Status |
| --- | --- | --- | --- | --- |
| 1 | [foundation.md](foundation.md) | 1 | Docs match the code | **COMPLETED** |
| 2 | [foundation.md](foundation.md) | 2 | A failed ledger write fails the request | **COMPLETED** |
| 3 | [guards.md](guards.md) | 1 | One schema and one entity builder | Open |
| 4 | [ux-standards.md](ux-standards.md) | 1 | One component kit | |
| 5 | [live-data.md](live-data.md) | 1–2 | Records live in PostgreSQL | |
| 6 | [mcp-apps.md](mcp-apps.md) | 4a | Process definitions are proposals | |
| 7 | [oscal-catalog.md](oscal-catalog.md) | 1 | The catalogs. Needs files from Johann | |
| 8 | [data-grid-parity.md](data-grid-parity.md) | open phases | Grid work | |
"""


def rows():
    return np.parse_rows(README)


class ParseTests(unittest.TestCase):
    def test_parses_every_table_row_and_skips_the_header(self):
        self.assertEqual([r.n for r in rows()], [1, 2, 3, 4, 5, 6, 7, 8])

    def test_status_is_the_last_cell_and_may_be_empty(self):
        r = {x.n: x for x in rows()}
        self.assertIn("COMPLETED", r[1].status)
        self.assertEqual(r[3].status, "Open")
        self.assertEqual(r[4].status, "")

    def test_phase_is_normalized_to_ascii(self):
        r = {x.n: x for x in rows()}
        self.assertEqual(r[5].phase, "1-2")
        self.assertEqual(r[6].phase, "4a")


class NextTests(unittest.TestCase):
    def test_next_is_the_first_row_not_completed(self):
        self.assertEqual(np.first_open(rows()).n, 3)

    def test_nothing_open_returns_none(self):
        done = [np.Row(1, "a", "1", "x", "COMPLETED")]
        self.assertIsNone(np.first_open(done))

    def test_a_row_with_a_needs_phrase_is_blocked(self):
        r = {x.n: x for x in rows()}
        self.assertIn("Needs", np.blocker(r[7]))

    def test_a_non_numeric_phase_is_blocked(self):
        r = {x.n: x for x in rows()}
        self.assertIsNotNone(np.blocker(r[8]))

    def test_an_ordinary_row_is_not_blocked(self):
        r = {x.n: x for x in rows()}
        self.assertIsNone(np.blocker(r[3]))
        self.assertIsNone(np.blocker(r[6]))


class CheckTests(unittest.TestCase):
    def check(self, brief, phase, strict=True):
        return np.check(rows(), brief, phase, strict)[0]

    def test_the_next_open_row_passes(self):
        self.assertEqual(self.check("guards", "1"), np.OK)

    def test_a_completed_row_is_refused(self):
        self.assertEqual(self.check("foundation", "1"), np.DONE)

    def test_an_unknown_row_is_refused(self):
        self.assertEqual(self.check("nope", "9"), np.UNKNOWN)

    def test_out_of_order_is_refused_when_strict(self):
        self.assertEqual(self.check("ux-standards", "1"), np.OUT_OF_ORDER)

    def test_out_of_order_is_allowed_when_not_strict(self):
        self.assertEqual(self.check("ux-standards", "1", strict=False), np.OK)

    def test_a_blocked_row_is_refused_even_when_not_strict(self):
        self.assertEqual(self.check("oscal-catalog", "1", strict=False), np.BLOCKED)

    def test_phase_input_with_en_dash_or_spaces_still_matches(self):
        self.assertEqual(np.norm_phase(" 1 – 2 "), "1-2")
        self.assertEqual(self.check("live-data", "1–2", strict=False), np.OK)


class RealReadmeTests(unittest.TestCase):
    def test_the_real_readme_parses_and_has_an_order_column_in_sequence(self):
        text = (Path(__file__).resolve().parents[2] / "docs" / "plans" / "README.md").read_text(encoding="utf-8")
        parsed = np.parse_rows(text)
        self.assertGreater(len(parsed), 100)
        self.assertEqual([r.n for r in parsed], list(range(1, len(parsed) + 1)))


if __name__ == "__main__":
    unittest.main()
