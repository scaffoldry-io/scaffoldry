#!/usr/bin/env python3
"""Tests for next-phase.py. Run with: python3 -m unittest discover -s governance/scripts -p "test_*.py"."""

import importlib.util
import os
import subprocess
import tempfile
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



PARALLEL = """| Order | Brief | Phase | What is true after it | Status |
| --- | --- | --- | --- | --- |
| 1 | [guards.md](guards.md) | 1 | One schema | **COMPLETED** |
| 2 | [ux-standards.md](ux-standards.md) | 1 | One component kit | Open |
| 3 | [jobs.md](jobs.md) | 1 | A job queue | Open |
| 4 | [jobs.md](jobs.md) | 2 | A scheduler | Open |
| 5 | [links.md](links.md) | 1 | Links by id | Open |
| 6 | [oscal-catalog.md](oscal-catalog.md) | 1 | Catalogs. Needs files from Johann | Open |
| 7 | [views.md](views.md) | 1 | Views | Open |
"""


def prows():
    return np.parse_rows(PARALLEL)


class ClaimParseTests(unittest.TestCase):
    def test_branches_become_claims_for_both_prefixes(self):
        refs = [
            "refs/heads/feat/jobs-phase-1",
            "refs/heads/agent/ux-standards-phase-1",
            "refs/heads/feat/links-phase-1-2",
            "refs/heads/main",
            "refs/heads/feat/not-a-claim",
        ]
        self.assertEqual(
            np.claims_from_refs(refs),
            {("jobs", "1"), ("ux-standards", "1"), ("links", "1-2")},
        )

    def test_a_claim_name_is_the_branch_name(self):
        self.assertEqual(np.claim_branch("jobs", "1"), "feat/jobs-phase-1")


class AvailabilityTests(unittest.TestCase):
    def test_with_no_claims_the_first_open_row_is_available(self):
        self.assertEqual(np.next_available(prows(), set()).n, 2)

    def test_a_claimed_row_is_skipped(self):
        self.assertEqual(np.next_available(prows(), {("ux-standards", "1")}).n, 3)

    def test_a_brief_with_work_in_flight_is_skipped_entirely(self):
        claims = {("ux-standards", "1"), ("jobs", "1")}
        self.assertEqual(np.next_available(prows(), claims).n, 5)

    def test_a_later_phase_waits_for_the_earlier_phase_of_its_brief(self):
        row4 = {r.n: r for r in prows()}[4]
        self.assertIn("jobs", np.claim_blocker(prows(), row4, set()))
        self.assertIn("phase 1", np.claim_blocker(prows(), row4, set()))

    def test_the_scan_stops_at_a_row_that_needs_a_person(self):
        claims = {("ux-standards", "1"), ("jobs", "1"), ("links", "1")}
        self.assertIsNone(np.next_available(prows(), claims, window=10))

    def test_the_scan_looks_at_only_three_open_rows(self):
        claims = {("ux-standards", "1"), ("jobs", "1")}
        # Rows 2, 3 and 4 are the first three open rows. Row 5 is the fourth, so it is out of reach.
        self.assertIsNone(np.next_available(prows(), claims, window=3))

    def test_claiming_a_claimed_row_is_refused(self):
        row2 = {r.n: r for r in prows()}[2]
        self.assertIn("already claimed", np.claim_blocker(prows(), row2, {("ux-standards", "1")}))

    def test_claiming_a_completed_row_is_refused(self):
        row1 = {r.n: r for r in prows()}[1]
        self.assertIn("COMPLETED", np.claim_blocker(prows(), row1, set()))

    def test_claims_on_completed_rows_are_ignored(self):
        # A leftover branch for a finished row must not block anything.
        self.assertEqual(np.live_claims(prows(), {("guards", "1"), ("jobs", "1")}), {("jobs", "1")})


class ClaimPushTests(unittest.TestCase):
    """The lock is the remote. Prove that a second creator is refused, against a real bare repo."""

    def git(self, *args, cwd):
        return subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()

    def test_only_the_first_push_creates_the_branch(self):
        with tempfile.TemporaryDirectory() as tmp:
            remote = Path(tmp, "remote.git")
            subprocess.run(["git", "init", "--bare", "-q", str(remote)], check=True)
            first = Path(tmp, "a")
            subprocess.run(["git", "clone", "-q", str(remote), str(first)], check=True, capture_output=True)
            self.git("config", "user.email", "a@example.test", cwd=first)
            self.git("config", "user.name", "A", cwd=first)
            Path(first, "f").write_text("x")
            self.git("add", "f", cwd=first)
            self.git("commit", "-q", "-m", "init", cwd=first)
            sha = self.git("rev-parse", "HEAD", cwd=first)
            old = Path.cwd()
            try:
                os.chdir(first)
                self.assertTrue(np.push_claim("origin", "feat/jobs-phase-1", sha))
                self.assertFalse(np.push_claim("origin", "feat/jobs-phase-1", sha))
                refs = np.remote_refs("origin")
            finally:
                os.chdir(old)
            self.assertEqual(np.claims_from_refs(refs), {("jobs", "1")})

if __name__ == "__main__":
    unittest.main()
