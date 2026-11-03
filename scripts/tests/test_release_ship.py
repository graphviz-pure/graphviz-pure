"""Tests for publish_github_release (run with `python3 -m unittest`).

The function is the only place a GitHub release is created. These tests record the
`gh` arguments a fake `gh` receives and never contact GitHub or run release-ship.sh,
which pushes tags.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

LIB = Path(__file__).resolve().parents[1] / "release-lib.sh"
FAKE_GH = """#!/usr/bin/env python3
import json, os, sys
args = sys.argv[1:]
with open(os.environ["GH_LOG"], "a") as fh:
    fh.write(json.dumps(args) + "\\n")
if args[:2] == ["release", "view"]:
    state = os.environ["GH_RELEASE_STATE"]
    if state == "missing":
        sys.exit(1)
    print("true" if state == "draft" else "false")
"""


class PublishGithubReleaseTest(unittest.TestCase):
    def publish(self, version, state):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            dist = work / "dist"
            dist.mkdir()
            asset = dist / "graphviz-pure.tar.gz"
            asset.write_bytes(b"asset")
            notes = work / "notes.md"
            notes.write_text("## notes\n")
            log = work / "gh.log"
            gh = work / "gh"
            gh.write_text(FAKE_GH)
            gh.chmod(0o755)
            env = os.environ.copy()
            for name in ("GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN"):
                env.pop(name, None)
            env.update(PATH=f"{work}{os.pathsep}{env.get('PATH', '')}", GH_LOG=str(log),
                       GH_RELEASE_STATE=state, VERSION=version, DIST=str(dist),
                       NOTES=str(notes), PYTHONDONTWRITEBYTECODE="1")
            script = f'''set -euo pipefail
. "{LIB}"
publish_github_release "$VERSION" "v$VERSION" "$DIST" "$NOTES"
'''
            result = subprocess.run(["bash", "-c", script], env=env, capture_output=True,
                                    text=True, check=False)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            return result.stdout, calls, asset, notes

    def commands(self, calls, name):
        return [args for args in calls if args[:2] == ["release", name]]

    def assert_marked(self, calls, flag):
        other = "--prerelease=false" if flag == "--prerelease=true" else "--prerelease=true"
        for name in ("create", "edit"):
            matched = self.commands(calls, name)
            self.assertEqual(len(matched), 1, calls)
            self.assertIn(flag, matched[0])
            self.assertNotIn(other, matched[0])
            self.assertIn("--repo", matched[0])
            self.assertIn("graphviz-pure/graphviz-pure", matched[0])

    def test_prerelease_versions_are_marked_on_create_and_publish(self):
        for version in ("1.0.0-rc.1", "0.3.0-alpha.2", "2.0.0-beta.10", "1.2.3-dev.1"):
            with self.subTest(version=version):
                stdout, calls, asset, notes = self.publish(version, "missing")
                self.assertEqual(stdout.strip(), f"published v{version}")
                self.assert_marked(calls, "--prerelease=true")
                create = self.commands(calls, "create")[0]
                self.assertIn("--draft", create)
                self.assertIn("--verify-tag", create)
                self.assertIn(f"graphviz-pure {version}", create)
                self.assertIn(str(notes), create)
                self.assertEqual(create[-1], str(asset))
                edit = self.commands(calls, "edit")[0]
                self.assertIn("--draft=false", edit)
                self.assertEqual(self.commands(calls, "upload"), [])

    def test_stable_versions_are_not_prereleases(self):
        for version in ("1.0.0", "0.2.3"):
            with self.subTest(version=version):
                stdout, calls, _, _ = self.publish(version, "missing")
                self.assertEqual(stdout.strip(), f"published v{version}")
                self.assert_marked(calls, "--prerelease=false")

    def test_republishing_a_draft_sets_the_flag(self):
        stdout, calls, asset, notes = self.publish("1.0.0-rc.1", "draft")
        self.assertEqual(stdout.strip(), "published the draft v1.0.0-rc.1")
        self.assertEqual(self.commands(calls, "create"), [])
        upload = self.commands(calls, "upload")
        self.assertEqual(len(upload), 1)
        self.assertIn("--clobber", upload[0])
        self.assertIn(str(asset), upload[0])
        edit = self.commands(calls, "edit")
        self.assertEqual(len(edit), 1)
        self.assertIn("--prerelease=true", edit[0])
        self.assertIn("--draft=false", edit[0])
        self.assertIn("--notes-file", edit[0])
        self.assertIn(str(notes), edit[0])

    def test_republishing_a_stable_draft_clears_the_flag(self):
        stdout, calls, _, _ = self.publish("1.0.0", "draft")
        self.assertEqual(stdout.strip(), "published the draft v1.0.0")
        edit = self.commands(calls, "edit")
        self.assertEqual(len(edit), 1)
        self.assertIn("--prerelease=false", edit[0])
        self.assertNotIn("--prerelease=true", edit[0])

    def test_published_release_is_left_unchanged(self):
        stdout, calls, _, _ = self.publish("1.0.0-rc.1", "published")
        self.assertEqual(stdout.strip(), "v1.0.0-rc.1 is already published")
        self.assertEqual(self.commands(calls, "create"), [])
        self.assertEqual(self.commands(calls, "edit"), [])
        self.assertEqual(self.commands(calls, "upload"), [])
        self.assertTrue(all(args[:2] == ["release", "view"] for args in calls))


class GhFlagSyntaxTest(unittest.TestCase):
    @unittest.skipUnless(shutil.which("gh"), "gh is not installed")
    def test_installed_gh_accepts_the_prerelease_flag_without_a_tag(self):
        # No tag: create and edit fail while parsing arguments, before any API call.
        env = os.environ.copy()
        for name in ("GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN"):
            env.pop(name, None)
        env["GH_FORCE_TTY"] = "0"
        for command in (
            ["gh", "release", "create", "--prerelease=true"],
            ["gh", "release", "create", "--prerelease=false"],
            ["gh", "release", "edit", "--prerelease=true"],
            ["gh", "release", "edit", "--prerelease=false"],
        ):
            with self.subTest(command=command):
                result = subprocess.run(command, env=env, capture_output=True, text=True,
                                        stdin=subprocess.DEVNULL, check=False)
                self.assertNotEqual(result.returncode, 0)
                combined = result.stdout + result.stderr
                self.assertNotIn("unknown flag", combined)
                self.assertNotRegex(combined, r"https://api.github.com")


if __name__ == "__main__":
    unittest.main()
