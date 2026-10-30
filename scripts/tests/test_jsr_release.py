"""JSR registry gates: exercise the verifier with deterministic registry responses."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
FLAGS = ["hasReadme", "hasReadmeExamples", "allEntrypointsDocs", "allFastCheck",
         "hasProvenance", "hasDescription", "atLeastOneRuntimeCompatible",
         "multipleRuntimesCompatible"]
FULL = dict.fromkeys(FLAGS, True) | {"percentageDocumentedSymbols": 1}


class JsrReleaseTest(unittest.TestCase):
    def verify(self, score, latest="1.1.0", retry=False, yanked=False):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            (work / "meta.json").write_text(json.dumps({
                "latest": latest, "versions": {"1.1.0": {"yanked": yanked}}}))
            (work / "score.json").write_text(json.dumps(score))
            (work / "full.json").write_text(json.dumps(FULL))
            (work / "curl").write_text('''#!/bin/bash
set -eu
url=$2
out=$4
case "$url" in
  */meta.json) cp "$FIXTURE/meta.json" "$out" ;;
  */score)
    if [ "$SCORE_RETRY" = 1 ] && [ -f "$FIXTURE/retried" ]; then
      cp "$FIXTURE/full.json" "$out"
    else
      cp "$FIXTURE/score.json" "$out"
    fi
    touch "$FIXTURE/retried" ;;
  *) exit 99 ;;
esac
''')
            (work / "sleep").write_text("#!/bin/sh\nexit 0\n")
            for name in ("curl", "sleep"):
                (work / name).chmod(0o755)
            env = os.environ | {"PATH": f"{work}:{os.environ['PATH']}",
                                "FIXTURE": directory, "SCORE_RETRY": str(int(retry))}
            return subprocess.run(
                [str(ROOT / "scripts/release-verify.sh"), "1.1.0", "--ci",
                 "--channels", "jsr", "--wait", "1" if retry else "0"],
                env=env, capture_output=True, text=True)

    def test_full_score(self):
        self.assertEqual(self.verify(FULL).returncode, 0)

    def test_each_score_item_and_missing_fields_fail(self):
        for key in FULL:
            for value in (False, None):
                with self.subTest(key=key, value=value):
                    score = FULL | {key: value}
                    if value is None:
                        del score[key]
                    result = self.verify(score)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(key, result.stdout)
        self.assertNotEqual(self.verify(FULL | {"percentageDocumentedSymbols": 0.99}).returncode, 0)

    def test_old_version_does_not_use_latest_score(self):
        self.assertEqual(self.verify({}, latest="1.2.0").returncode, 0)

    def test_yanked_version_still_fails(self):
        self.assertNotEqual(self.verify(FULL, yanked=True).returncode, 0)

    def test_score_can_arrive_after_publish(self):
        self.assertEqual(self.verify({}, retry=True).returncode, 0)

    def test_license_expressions_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            file = Path(directory) / "jsr.json"
            for license, valid in [("EPL-2.0", True), ("EPL-2.0 AND MIT", False),
                                   ("(MIT)", False), ("MIT OR Apache-2.0", False),
                                   (None, False)]:
                file.write_text(json.dumps({"license": license}))
                result = subprocess.run(["node", str(ROOT / "scripts/check-jsr-license.mjs"), str(file)],
                                        capture_output=True)
                self.assertEqual(result.returncode == 0, valid)
