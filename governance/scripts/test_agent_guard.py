#!/usr/bin/env python3
"""Tests for agent-guard.py. Run with: python3 -m unittest discover -s governance/scripts -p "test_*.py"."""

import importlib.util
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("agent_guard", Path(__file__).with_name("agent-guard.py"))
ag = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ag)


class BranchTests(unittest.TestCase):
    def test_parses_brief_and_phase(self):
        self.assertEqual(ag.parse_branch("agent/guards-phase-1"), ("guards", "1"))
        self.assertEqual(ag.parse_branch("agent/mcp-apps-phase-4a"), ("mcp-apps", "4a"))
        self.assertEqual(ag.parse_branch("agent/live-data-phase-1-2"), ("live-data", "1-2"))

    def test_rejects_other_branches(self):
        self.assertIsNone(ag.parse_branch("feat/guards-phase-1"))
        self.assertIsNone(ag.parse_branch("agent/whatever"))


class PathTests(unittest.TestCase):
    def v(self, changes, deps=False):
        return ag.check_paths(changes, deps)

    def test_ordinary_code_and_test_changes_pass(self):
        self.assertEqual(self.v([("M", "crates/scaffoldry-server/src/state.rs"), ("A", "crates/scaffoldry-server/tests/x_test.rs")]), [])

    def test_any_change_under_github_or_devcontainer_fails(self):
        self.assertTrue(self.v([("M", ".github/workflows/ci.yml")]))
        self.assertTrue(self.v([("A", ".github/workflows/new.yml")]))
        self.assertTrue(self.v([("M", ".devcontainer/Dockerfile")]))

    def test_agents_and_license_are_protected(self):
        self.assertTrue(self.v([("M", "AGENTS.md")]))
        self.assertTrue(self.v([("M", "LICENSE")]))

    def test_existing_audit_scripts_cannot_be_modified_or_deleted_but_new_scripts_may_be_added(self):
        self.assertTrue(self.v([("M", "governance/scripts/audit-licenses.py")]))
        self.assertTrue(self.v([("D", "governance/scripts/audit-architecture.py")]))
        self.assertTrue(self.v([("M", "governance/scripts/next-phase.py")]))
        self.assertEqual(self.v([("A", "governance/scripts/validate-manifest.py")]), [])

    def test_dependency_files_fail_unless_approved(self):
        for p in ("Cargo.toml", "Cargo.lock", "crates/scaffoldry-server/Cargo.toml", "package.json", "package-lock.json", "apps/web/package.json"):
            self.assertTrue(self.v([("M", p)]), p)
            self.assertEqual(self.v([("M", p)], deps=True), [], p)


def diff(file, removed=(), added=()):
    out = [f"diff --git a/{file} b/{file}", f"--- a/{file}", f"+++ b/{file}", "@@ -1 +1 @@"]
    out += [f"-{x}" for x in removed] + [f"+{x}" for x in added]
    return "\n".join(out) + "\n"


class ChangedLinesTests(unittest.TestCase):
    def test_groups_removed_and_added_lines_by_file_and_ignores_headers(self):
        d = diff("a.txt", ["old"], ["new"]) + diff("b.txt", [], ["x", "y"])
        got = ag.parse_changed_lines(d)
        self.assertEqual(got["a.txt"], (["old"], ["new"]))
        self.assertEqual(got["b.txt"], ([], ["x", "y"]))


class ReadmeTests(unittest.TestCase):
    OLD = "| 3 | [guards.md](guards.md) | 1 | One schema and one builder | Open |"
    NEW = "| 3 | [guards.md](guards.md) | 1 | One schema and one builder | **COMPLETED** |"

    def test_flipping_the_status_of_the_branch_row_passes(self):
        self.assertEqual(ag.check_readme([self.OLD], [self.NEW], "guards", "1"), [])

    def test_changing_the_description_fails(self):
        bad = self.NEW.replace("One schema", "Everything")
        self.assertTrue(ag.check_readme([self.OLD], [bad], "guards", "1"))

    def test_changing_a_row_of_another_brief_fails(self):
        self.assertTrue(ag.check_readme([self.OLD], [self.NEW], "jobs", "1"))

    def test_adding_or_removing_lines_fails(self):
        self.assertTrue(ag.check_readme([self.OLD], [self.NEW, "extra"], "guards", "1"))
        self.assertTrue(ag.check_readme([self.OLD, "x"], [self.NEW], "guards", "1"))

    def test_non_table_lines_fail(self):
        self.assertTrue(ag.check_readme(["# Title"], ["# New title"], "guards", "1"))


class PlanEditTests(unittest.TestCase):
    def test_only_the_status_line_of_a_brief_may_change(self):
        ok = {"docs/plans/guards.md": (["> **Status: Open**"], ["> **Status: COMPLETED**"])}
        self.assertEqual(ag.check_plan_edits(ok), [])
        bad = {"docs/plans/guards.md": (["Do not add a dependency."], ["Add what you like."])}
        self.assertTrue(ag.check_plan_edits(bad))

    def test_the_readme_is_handled_elsewhere(self):
        self.assertEqual(ag.check_plan_edits({"docs/plans/README.md": (["a"], ["b"])}), [])


class TestIntegrityTests(unittest.TestCase):
    def test_fewer_test_markers_than_before_fails(self):
        d = {"crates/x/tests/a_test.rs": (["#[test]", "fn a() {}"], ["fn a() {}"])}
        self.assertTrue(ag.check_tests(d))

    def test_replacing_a_test_passes(self):
        d = {"crates/x/tests/a_test.rs": (["#[test]"], ["#[test]", "#[test]"])}
        self.assertEqual(ag.check_tests(d), [])

    def test_js_skip_and_only_fail(self):
        d = {"apps/web/src/test/a.test.tsx": ([], ["  it.skip('x', () => {})"])}
        self.assertTrue(ag.check_tests(d))
        d = {"apps/web/src/test/a.test.tsx": ([], ["  it.only('x', () => {})"])}
        self.assertTrue(ag.check_tests(d))

    def test_ignore_attribute_fails_except_for_the_million_row_test(self):
        d = {"crates/x/tests/a_test.rs": ([], ["#[ignore]", "fn slow() {}"])}
        self.assertTrue(ag.check_tests(d))
        d = {"crates/x/tests/row_scale_test.rs": ([], ["#[ignore]", "fn million_rows() {}"])}
        self.assertEqual(ag.check_tests(d), [])

    def test_non_test_files_are_not_scanned(self):
        d = {"crates/x/src/lib.rs": (["#[test]"], [])}
        self.assertEqual(ag.check_tests(d), [])


class GitIntegrationTests(unittest.TestCase):
    """Runs the real script against a throwaway repository, so the git parsing is exercised."""

    def setUp(self):
        import subprocess, tempfile
        self.sp = subprocess
        self.dir = tempfile.TemporaryDirectory()
        self.root = Path(self.dir.name)
        self.git("init", "-q", "-b", "main")
        self.git("config", "user.email", "t@example.com")
        self.git("config", "user.name", "t")
        (self.root / "docs/plans").mkdir(parents=True)
        (self.root / "docs/plans/README.md").write_text(
            "| Order | Brief | Phase | What | Status |\n| --- | --- | --- | --- | --- |\n"
            "| 1 | [guards.md](guards.md) | 1 | One schema | Open |\n")
        (self.root / "docs/plans/guards.md").write_text("> **Status: Open**\n\nBody.\n")
        (self.root / "crates").mkdir()
        (self.root / "crates/a_test.rs").write_text("#[test]\nfn a() {}\n")
        (self.root / "Cargo.toml").write_text("[workspace]\n")
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "base")
        self.git("update-ref", "refs/remotes/origin/main", "HEAD")
        self.git("switch", "-q", "-c", "agent/guards-phase-1")

    def tearDown(self):
        self.dir.cleanup()

    def git(self, *a):
        return self.sp.run(["git", *a], cwd=self.root, check=True, capture_output=True, text=True).stdout

    def guard(self, *extra, branch="agent/guards-phase-1"):
        script = Path(__file__).with_name("agent-guard.py")
        r = self.sp.run(["python3", str(script), "--branch", branch, "--base", "origin/main", "--head", "HEAD", *extra],
                        cwd=self.root, capture_output=True, text=True)
        return r.returncode, r.stdout + r.stderr

    def commit(self, msg="work"):
        self.git("add", "-A")
        self.git("commit", "-q", "-m", msg)

    def test_a_clean_phase_passes(self):
        (self.root / "crates/b.rs").write_text("fn b() {}\n")
        (self.root / "crates/a_test.rs").write_text("#[test]\nfn a() {}\n#[test]\nfn b() {}\n")
        readme = self.root / "docs/plans/README.md"
        readme.write_text(readme.read_text().replace("| Open |", "| **COMPLETED** |"))
        (self.root / "docs/plans/guards.md").write_text("> **Status: COMPLETED**\n\nBody.\n")
        self.commit()
        code, out = self.guard()
        self.assertEqual(code, 0, out)

    def test_editing_ci_fails(self):
        (self.root / ".github/workflows").mkdir(parents=True)
        (self.root / ".github/workflows/ci.yml").write_text("name: x\n")
        self.commit()
        code, out = self.guard()
        self.assertEqual(code, 1)
        self.assertIn(".github/workflows/ci.yml", out)

    def test_a_dependency_change_fails_then_passes_with_the_flag(self):
        (self.root / "Cargo.toml").write_text("[workspace]\n[dependencies]\nx = \"1\"\n")
        self.commit()
        self.assertEqual(self.guard()[0], 1)
        self.assertEqual(self.guard("--deps-approved")[0], 0)

    def test_deleting_a_test_fails(self):
        (self.root / "crates/a_test.rs").write_text("fn a() {}\n")
        self.commit()
        code, out = self.guard()
        self.assertEqual(code, 1)
        self.assertIn("fewer tests", out)

    def test_editing_a_plan_body_fails(self):
        (self.root / "docs/plans/guards.md").write_text("> **Status: Open**\n\nDifferent body.\n")
        self.commit()
        self.assertEqual(self.guard()[0], 1)

    def test_a_non_agent_branch_is_not_checked(self):
        (self.root / ".github").mkdir()
        (self.root / ".github/x.yml").write_text("a\n")
        self.commit()
        code, out = self.guard(branch="feat/anything")
        self.assertEqual(code, 0)
        self.assertIn("Not an agent branch", out)


if __name__ == "__main__":
    unittest.main()
