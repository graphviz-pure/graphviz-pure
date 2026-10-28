"""Exercise the label gate with the pinned tool's exit-code contract, without compiling Rust."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "check-semver.sh"
PACKAGES = ["graphviz-pure", "graphviz-pure-raster"]


class SemverGateTest(unittest.TestCase):
    def run_gate(self, statuses, breaking=False, versions=None, baseline=None, mode="pr",
                 release="0.3.0", manifest=None, merged_breaking=None):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            git = work / "git"
            git.write_text("""#!/usr/bin/env python3
import json, os, sys
with open(os.environ['GIT_CALLS'], 'a') as calls:
    calls.write(json.dumps(sys.argv[1:]) + '\\n')
if sys.argv[1] == 'merge-base':
    print('a' * 40)
elif sys.argv[1] == 'log':
    assert sys.argv[-1].startswith('v') and '-rc' not in sys.argv[-1]
    print('c' * 40 + '\\0API change (#1)')
else:
    sys.exit(2)
""")
            git.chmod(0o755)
            gh = work / "gh"
            gh.write_text("""#!/usr/bin/env python3
import json, os
labels = [{'name': 'breaking'}] if os.environ['MERGED_BREAKING'] == 'true' else []
print(json.dumps({'data': {'repository': {'p1': {
    'number': 1, 'title': 'API change', 'url': 'https://example.test/pr/1',
    'body': '## Changelog entry\\nNot user-visible.', 'labels': {'nodes': labels}
}}}}))
""")
            gh.chmod(0o755)
            cargo = work / "cargo"
            cargo.write_text("""#!/usr/bin/env python3
import json, os, sys
if sys.argv[1:] == ['semver-checks', '--version']:
    print('cargo-semver-checks 0.51.0')
    sys.exit(0)
with open(os.environ['CALLS'], 'a') as calls:
    calls.write(json.dumps(sys.argv[1:]) + '\\n')
package = sys.argv[sys.argv.index('-p') + 1]
sys.exit(json.loads(os.environ['STATUSES'])[package])
""")
            cargo.chmod(0o755)
            curl = work / "curl"
            curl.write_text("#!/usr/bin/env python3\nimport os\nassert os.environ['MODE'] == 'release'\nprint(os.environ['VERSIONS'])\n")
            curl.chmod(0o755)
            env = dict(os.environ, MODE=mode, PATH=f"{work}:{os.environ['PATH']}",
                       GIT_CALLS=str(work / "git_calls"),
                       MERGED_BREAKING=str(breaking if merged_breaking is None else merged_breaking).lower(),
                       BREAKING=str(breaking).lower(), STATUSES=json.dumps(dict(zip(PACKAGES, statuses))),
                       VERSIONS="\n".join(json.dumps(version) for version in
                                          (versions or [{"vers": "0.2.3", "yanked": False}])),
                       CALLS=str(work / "calls"))
            for name in ("BASELINE_VERSION", "BASE_SHA", "RELEASE_VERSION", "SEMVER_MANIFEST"):
                env.pop(name, None)
            if mode == "pr":
                env["BASE_SHA"] = "b" * 40
            else:
                env["RELEASE_VERSION"] = release
            if manifest:
                env["SEMVER_MANIFEST"] = manifest
            if baseline:
                env["BASELINE_VERSION"] = baseline
            result = subprocess.run(["bash", str(SCRIPT), mode], env=env, text=True,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)
            calls = work / "calls"
            git_calls = work / "git_calls"
            self.git_calls = [json.loads(line) for line in git_calls.read_text().splitlines()] if git_calls.exists() else []
            return result, [json.loads(line) for line in calls.read_text().splitlines()] if calls.exists() else []

    def test_compatible_pr_passes_without_label(self):
        result, calls = self.run_gate([0, 0])
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual([call[call.index("-p") + 1] for call in calls], PACKAGES)
        for call in calls:
            self.assertEqual(call[call.index("--baseline-rev") + 1], "a" * 40)
            self.assertNotIn("--baseline-version", call)
            self.assertEqual(call[call.index("--release-type") + 1], "minor")

    def test_behind_branch_uses_merge_base_instead_of_upstream_tip(self):
        result, calls = self.run_gate([0, 0])
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn(["merge-base", "b" * 40, "HEAD"], self.git_calls)
        self.assertTrue(all(call[call.index("--baseline-rev") + 1] == "a" * 40 for call in calls))

    def test_release_labels_cover_the_published_baseline_not_the_rc(self):
        result, _ = self.run_gate([100, 0], mode="release", breaking=False, merged_breaking=True)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertTrue(all(call[-1] == "v0.2.3..HEAD" for call in self.git_calls))

    def test_break_in_either_stable_crate_requires_label(self):
        for statuses in ([100, 0], [0, 100]):
            with self.subTest(statuses=statuses):
                result, calls = self.run_gate(statuses)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("Add the breaking label", result.stdout)
                self.assertEqual(len(calls), 2)

    def test_adding_label_accepts_breakage_and_removing_it_fails(self):
        for breaking in (False, True, False):
            result, _ = self.run_gate([100, 100], breaking=breaking)
            self.assertEqual(result.returncode == 0, breaking, result.stdout)

    def test_label_does_not_waive_operational_errors(self):
        for status in (1, 101, 137):
            with self.subTest(status=status):
                result, calls = self.run_gate([status, 100], breaking=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("cannot waive", result.stdout)
                self.assertEqual(len(calls), 2)

    def test_baseline_advances_to_one_point_zero(self):
        result, calls = self.run_gate([0, 0], mode="release", versions=[
            {"vers": "0.2.3", "yanked": False},
            {"vers": "1.0.0", "yanked": False},
            {"vers": "1.1.0-rc.1", "yanked": False},
            {"vers": "2.0.0", "yanked": True},
        ])
        self.assertEqual(result.returncode, 0, result.stdout)
        for call in calls:
            self.assertEqual(call[call.index("--baseline-version") + 1], "1.0.0")
            # A manifest version bump cannot opt out of breaking-change lints, including 1.0.
            self.assertEqual(call[call.index("--release-type") + 1], "minor")

    def test_baseline_uses_numeric_semver_order(self):
        result, calls = self.run_gate([0, 0], mode="release", versions=[
            {"vers": version, "yanked": False} for version in ("1.9.0", "1.10.0", "0.99.0")
        ])
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertTrue(all(call[call.index("--baseline-version") + 1] == "1.10.0" for call in calls))

    def test_no_stable_baseline_fails_closed(self):
        result, calls = self.run_gate([0, 0], mode="release", breaking=True, versions=[
            {"vers": "1.0.0-rc.1", "yanked": False},
        ])
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [])

    def test_local_baseline_override(self):
        result, calls = self.run_gate([0, 0], mode="release", baseline="0.2.3")
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertTrue(all(call[call.index("--baseline-version") + 1] == "0.2.3" for call in calls))

    def test_release_break_requires_a_merged_breaking_label(self):
        result, _ = self.run_gate([100, 0], mode="release", release="1.0.0")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no merged PR carries breaking", result.stdout)

    def test_release_bump_must_cover_detected_breakage(self):
        for baseline, release, allowed in (
            ("0.2.3", "0.2.4", False), ("0.2.3", "0.3.0", True),
            ("0.2.3", "1.0.0", True), ("1.0.0", "1.0.1", False),
            ("1.0.0", "1.1.0", False), ("1.0.0", "2.0.0", True),
        ):
            with self.subTest(baseline=baseline, release=release):
                result, _ = self.run_gate([100, 0], mode="release", baseline=baseline,
                                          release=release, breaking=True)
                self.assertEqual(result.returncode == 0, allowed, result.stdout)
                required = "minor" if baseline.startswith("0.") else "major"
                self.assertIn(f"requires a {required} version bump", result.stdout)

    def test_release_compatible_without_breaking_label(self):
        result, _ = self.run_gate([0, 0], mode="release", release="0.2.4")
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_release_tool_error_is_not_waived(self):
        result, _ = self.run_gate([101, 0], mode="release", breaking=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cannot waive", result.stdout)

    def test_release_checks_the_recorded_master_snapshot(self):
        result, calls = self.run_gate([0, 0], mode="release", manifest="release-source/Cargo.toml")
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertTrue(all(call[call.index("--manifest-path") + 1] == "release-source/Cargo.toml"
                            for call in calls))


if __name__ == "__main__":
    unittest.main()
