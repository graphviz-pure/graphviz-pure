"""CI verification must report every channel, including staging and lookup failures."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class ReleaseVerifyCiTest(unittest.TestCase):
    def verify(self, status="404", transport=0, github="success", jsr="success"):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scripts").mkdir()
            for name in ("release-verify-ci.sh", "release-verify.sh", "release-lib.sh"):
                shutil.copy2(ROOT / "scripts" / name, root / "scripts" / name)
            (root / "curl").write_text('''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
args = sys.argv[1:]
if "-w" in args:
    print(os.environ["HTTP_STATUS"], end="")
    sys.exit(int(os.environ["TRANSPORT"]))
url = args[1]
file = Path(args[args.index("-o") + 1])
if url.endswith("meta.json"):
    value = {"latest": "1.1.0", "versions": {"1.1.0": {"yanked": False}}}
elif url.endswith("/score"):
    value = dict.fromkeys(["hasReadme", "hasReadmeExamples", "allEntrypointsDocs",
        "allFastCheck", "hasProvenance", "hasDescription", "atLeastOneRuntimeCompatible",
        "multipleRuntimesCompatible"], True) | {"percentageDocumentedSymbols": 1}
else:
    raise SystemExit("unexpected network request: " + url)
file.write_text(json.dumps(value))
''')
            (root / "curl").chmod(0o755)
            env = os.environ | dict(
                PATH=f"{root}:{os.environ['PATH']}", HTTP_STATUS=status, TRANSPORT=str(transport),
                VERSION="1.1.0", CRATES="skipped", GITHUB=github,
                RESULTS=f"github={github} crates=skipped npm=success jsr={jsr} ghcr=skipped wasmer=skipped pypi=skipped",
                GITHUB_OUTPUT=str(root / "outputs"), GITHUB_STEP_SUMMARY=str(root / "summary"))
            result = subprocess.run([str(root / "scripts/release-verify-ci.sh")],
                                    env=env, text=True, capture_output=True)
            summary = (root / "summary").read_text()
            output = (root / "outputs").read_text() if (root / "outputs").exists() else ""
            return result, summary, output

    def test_pending_with_no_live_or_failed_channels(self):
        result, summary, output = self.verify(github="skipped", jsr="skipped")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("npm_pending=true", output)
        self.assertIn("::notice::", result.stdout)
        self.assertIn("npm is staged and awaiting approval", summary)
        self.assertNotIn("All channels pass", summary)
        self.assertNotIn("jsr:", summary)

    def test_pending_still_checks_live_and_failed_channels(self):
        result, summary, _ = self.verify(github="failure")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("github: FAIL", summary)
        self.assertIn("[x] jsr:", summary)
        self.assertIn("npm: staged", summary)

    def test_lookup_errors_keep_other_results(self):
        for status, transport in [("503", 0), ("000", 7)]:
            with self.subTest(status=status, transport=transport):
                result, summary, output = self.verify(status, transport, github="failure")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("::error::npm version lookup", result.stdout)
                self.assertIn("github: FAIL", summary)
                self.assertIn("[x] jsr:", summary)
                self.assertIn("npm: FAIL", summary)
                self.assertNotIn("npm_pending", output)

    def test_live_npm_is_selected_for_full_verification(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scripts").mkdir()
            shutil.copy2(ROOT / "scripts/release-verify-ci.sh", root / "scripts/release-verify-ci.sh")
            # Record the real wrapper's arguments; the full registry verifier has its own tests.
            (root / "scripts/release-verify.sh").write_text('#!/bin/sh\nprintf "%s\\n" "$@"\n')
            (root / "scripts/release-verify.sh").chmod(0o755)
            (root / "curl").write_text('#!/bin/sh\nprintf 200\n')
            (root / "curl").chmod(0o755)
            env = os.environ | dict(PATH=f"{root}:{os.environ['PATH']}", VERSION="1.1.0",
                CRATES="skipped", GITHUB="failure", RESULTS="github=failure npm=success",
                GITHUB_OUTPUT=str(root / "outputs"), GITHUB_STEP_SUMMARY=str(root / "summary"))
            result = subprocess.run([str(root / "scripts/release-verify-ci.sh")],
                                    env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("--channels\nnpm\n--failed\ngithub\n", result.stdout)
            self.assertNotIn("--npm-staged", result.stdout)

    def test_explicit_empty_channels_only_reports_failures(self):
        result = subprocess.run([str(ROOT / "scripts/release-verify.sh"), "1.1.0", "--ci",
                                 "--channels", "", "--failed", "github"],
                                text=True, capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("github: FAIL", result.stdout)
        self.assertNotIn("jsr:", result.stdout)

    def test_prepare_actor_guard(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        prepare = workflow.split("  prepare:\n", 1)[1].split("\n  publish:", 1)[0]
        self.assertIn("retention-days: 90", prepare)
        guard = prepare.split("      - name: Require the owner's dispatch for JSR\n", 1)[1]
        self.assertIn("        if: inputs.dry_run != true\n", guard)
        shell = guard.split("        run: |\n", 1)[1].split("      - name:", 1)[0]
        shell = "\n".join(line[10:] for line in shell.splitlines())
        self.assertLess(prepare.index("Require the owner's dispatch"), prepare.index("Readiness, changelog"))
        for actor, expected, passes in [("Owner", "owner", True), ("owner", "OWNER", True),
                                        ("app[bot]", "owner", False), ("owner", "", False),
                                        ("other", "owner", False), ("APP[BOT]", "app[bot]", False)]:
            with self.subTest(actor=actor, expected=expected):
                result = subprocess.run(["bash", "-e", "-c", shell], capture_output=True,
                                        env=os.environ | dict(ACTOR=actor, EXPECTED_ACTOR=expected))
                self.assertEqual(result.returncode == 0, passes)
