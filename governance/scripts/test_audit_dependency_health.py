#!/usr/bin/env python3
"""Tests for audit-dependency-health.py. Run with: python3 -m unittest discover -s governance/scripts -p "test_*.py".

No test reaches the network. The registry answers are small hand-written documents in the registries' own shapes.
"""

import importlib.util
import json
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("audit_dependency_health", Path(__file__).with_name("audit-dependency-health.py"))
dh = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(dh)

NOW = datetime(2026, 10, 26, tzinfo=timezone.utc)


def days_ago(n):
    return datetime.fromtimestamp(NOW.timestamp() - n * 86400, tz=timezone.utc)


class ManifestTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.root = Path(self.dir.name)

    def tearDown(self):
        self.dir.cleanup()

    def write(self, path, text):
        p = self.root / path
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)

    def test_direct_dependencies_from_both_ecosystems(self):
        self.write("Cargo.toml", '[workspace]\nmembers = ["crates/a"]\n\n[workspace.dependencies]\nserde = "1.0"\n')
        self.write(
            "crates/a/Cargo.toml",
            '[package]\nname = "a"\n\n[dependencies]\nserde.workspace = true\nb = { path = "../b" }\n'
            'axum = { version = "0.8" }\n\n[dev-dependencies]\ntempfile = "3"\n',
        )
        self.write("package.json", json.dumps({"devDependencies": {"playwright-core": "^1"}}))
        self.write("apps/web/package.json", json.dumps({"dependencies": {"react": "^19"}, "devDependencies": {"vite": "^6"}}))
        self.write("apps/web/node_modules/left-pad/package.json", json.dumps({"dependencies": {"nothing": "1"}}))
        self.assertEqual(
            dh.direct_dependencies(self.root),
            [("crates", "axum"), ("crates", "serde"), ("crates", "tempfile"), ("npm", "playwright-core"), ("npm", "react"), ("npm", "vite")],
        )

    def test_a_workspace_reference_is_not_a_second_dependency_and_a_path_is_ours(self):
        self.write("Cargo.toml", '[workspace.dependencies]\nserde = "1.0"\nown = { path = "crates/own" }\n')
        self.write("crates/x/Cargo.toml", '[dependencies]\nserde = { workspace = true }\n')
        self.assertEqual(dh.direct_dependencies(self.root), [("crates", "serde")])


class ClassifyTests(unittest.TestCase):
    def test_a_recent_release_is_ok(self):
        self.assertEqual(dh.classify(days_ago(30), None, False, NOW)[0], "ok")

    def test_a_year_without_a_release_is_stale(self):
        status, reason = dh.classify(days_ago(400), None, False, NOW)
        self.assertEqual(status, "stale")
        self.assertIn("400 days", reason)

    def test_two_years_without_a_release_is_frozen(self):
        self.assertEqual(dh.classify(days_ago(800), None, False, NOW)[0], "frozen")

    def test_a_deprecated_latest_is_frozen_whatever_its_age(self):
        status, reason = dh.classify(days_ago(5), "Use other-pkg instead", False, NOW)
        self.assertEqual(status, "frozen")
        self.assertIn("Use other-pkg instead", reason)

    def test_a_yanked_latest_is_frozen(self):
        self.assertEqual(dh.classify(days_ago(5), None, True, NOW)[0], "frozen")

    def test_a_tighter_limit_applies_to_a_named_package(self):
        self.assertEqual(dh.classify(days_ago(150), None, False, NOW, stale_days=120)[0], "stale")
        self.assertEqual(dh.classify(days_ago(150), None, False, NOW)[0], "ok")

    def test_no_release_date_is_unknown_not_ok(self):
        self.assertEqual(dh.classify(None, None, False, NOW)[0], "unknown")


class RegistryShapeTests(unittest.TestCase):
    def test_npm_facts_read_the_latest_tag(self):
        doc = {
            "dist-tags": {"latest": "2.0.0", "next": "3.0.0-rc.1"},
            "time": {"2.0.0": "2026-01-02T03:04:05.000Z", "3.0.0-rc.1": "2026-10-01T00:00:00.000Z"},
            "versions": {"2.0.0": {"deprecated": "No longer maintained"}, "3.0.0-rc.1": {}},
        }
        version, released, deprecated, yanked = dh.npm_facts(doc)
        self.assertEqual(version, "2.0.0")
        self.assertEqual(released, datetime(2026, 1, 2, 3, 4, 5, tzinfo=timezone.utc))
        self.assertEqual(deprecated, "No longer maintained")
        self.assertFalse(yanked)

    def test_crate_facts_read_the_newest_stable_version(self):
        doc = {
            "crate": {"max_stable_version": "1.0.229"},
            "versions": [
                {"num": "2.0.0-alpha.1", "created_at": "2026-10-01T00:00:00Z", "yanked": False},
                {"num": "1.0.229", "created_at": "2026-07-18T23:05:13.266456Z", "yanked": True},
            ],
        }
        version, released, deprecated, yanked = dh.crate_facts(doc)
        self.assertEqual(version, "1.0.229")
        self.assertEqual(released.date().isoformat(), "2026-07-18")
        self.assertIsNone(deprecated)
        self.assertTrue(yanked)

    def test_a_crate_with_only_yanked_versions_has_no_stable_version(self):
        doc = {"crate": {"max_stable_version": None}, "versions": [{"num": "0.1.0", "created_at": "2020-01-01T00:00:00Z", "yanked": True}]}
        version, released, _, yanked = dh.crate_facts(doc)
        self.assertIsNone(version)
        self.assertTrue(yanked)


if __name__ == "__main__":
    unittest.main()
