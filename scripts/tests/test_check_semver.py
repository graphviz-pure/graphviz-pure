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
    def run_gate(self, statuses, breaking=False, versions=None, baseline=None):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            cargo = work / "cargo"
            cargo.write_text("""#!/usr/bin/env python3
import json, os, sys
with open(os.environ['CALLS'], 'a') as calls:
    calls.write(json.dumps(sys.argv[1:]) + '\\n')
package = sys.argv[sys.argv.index('-p') + 1]
sys.exit(json.loads(os.environ['STATUSES'])[package])
""")
            cargo.chmod(0o755)
            curl = work / "curl"
            curl.write_text("#!/usr/bin/env python3\nimport os\nprint(os.environ['VERSIONS'])\n")
            curl.chmod(0o755)
            env = dict(os.environ, PATH=f"{work}:{os.environ['PATH']}",
                       BREAKING=str(breaking).lower(), STATUSES=json.dumps(dict(zip(PACKAGES, statuses))),
                       VERSIONS="\n".join(json.dumps(version) for version in
                                          (versions or [{"vers": "0.2.3", "yanked": False}])),
                       CALLS=str(work / "calls"))
            env.pop("BASELINE_VERSION", None)
            if baseline:
                env["BASELINE_VERSION"] = baseline
            result = subprocess.run(["bash", str(SCRIPT)], env=env, text=True,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)
            calls = work / "calls"
            return result, [json.loads(line) for line in calls.read_text().splitlines()] if calls.exists() else []

    def test_compatible_pr_passes_without_label(self):
        result, calls = self.run_gate([0, 0])
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual([call[call.index("-p") + 1] for call in calls], PACKAGES)

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
        result, calls = self.run_gate([0, 0], versions=[
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
        result, calls = self.run_gate([0, 0], versions=[
            {"vers": version, "yanked": False} for version in ("1.9.0", "1.10.0", "0.99.0")
        ])
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertTrue(all(call[call.index("--baseline-version") + 1] == "1.10.0" for call in calls))

    def test_no_stable_baseline_fails_closed(self):
        result, calls = self.run_gate([0, 0], breaking=True, versions=[
            {"vers": "1.0.0-rc.1", "yanked": False},
        ])
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [])

    def test_local_baseline_override(self):
        result, calls = self.run_gate([0, 0], baseline="0.2.3")
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertTrue(all(call[call.index("--baseline-version") + 1] == "0.2.3" for call in calls))


if __name__ == "__main__":
    unittest.main()
