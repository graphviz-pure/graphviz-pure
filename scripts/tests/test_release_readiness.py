"""Readiness integration tests: real git/tag history, mocked GitHub and API checker I/O."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


SCRIPTS = Path(__file__).resolve().parents[1]


class ReleaseReadinessTest(unittest.TestCase):
    def test_pr_behind_upstream_ignores_upstream_only_api_additions(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            (repo / "scripts").mkdir()
            shutil.copy2(SCRIPTS / "check-semver.sh", repo / "scripts/check-semver.sh")
            env = dict(os.environ)
            for name in ("GIT_DIR", "GIT_WORK_TREE", "GIT_COMMON_DIR"):
                env.pop(name, None)

            def git(*args):
                return subprocess.check_output(
                    ["git", "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgSign=false", *args],
                    cwd=repo, env=env, text=True, stderr=subprocess.DEVNULL).strip()

            git("init", "-q")
            git("config", "user.name", "Test")
            git("config", "user.email", "test@example.invalid")
            (repo / "api").write_text("old_api\n")
            git("add", ".")
            git("commit", "-qm", "Shared ancestor")
            ancestor = git("rev-parse", "HEAD")
            git("checkout", "-qb", "upstream")
            (repo / "api").write_text("old_api\nnew_api\n")
            git("commit", "-qam", "Upstream API addition")
            upstream = git("rev-parse", "HEAD")
            git("checkout", "-qb", "pr", ancestor)
            (repo / "docs").write_text("Only this PR's documentation\n")
            git("add", ".")
            git("commit", "-qm", "Docs-only PR behind upstream")
            cargo = repo / "cargo"
            cargo.write_text('''#!/usr/bin/env python3
import pathlib, subprocess, sys
if sys.argv[1:] == ['semver-checks', '--version']:
    print('cargo-semver-checks 0.51.0')
    sys.exit(0)
baseline = sys.argv[sys.argv.index('--baseline-rev') + 1]
old = subprocess.check_output(['git', 'show', baseline + ':api'], text=True)
sys.exit(0 if old == pathlib.Path('api').read_text() else 100)
''')
            cargo.chmod(0o755)
            env.update(PATH=f"{repo}:{env['PATH']}", BASE_SHA=upstream, BREAKING="false")
            result = subprocess.run(["bash", "scripts/check-semver.sh", "pr"], cwd=repo,
                                    env=env, capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn(ancestor, result.stdout)

    def run_readiness(self, version, rc=False, tool_error=False):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            repo, binaries = work / "repo", work / "bin"
            (repo / "scripts").mkdir(parents=True)
            binaries.mkdir()
            for name in ("release-check.sh", "release-lib.sh", "release-changelog.py", "check-semver.sh"):
                shutil.copy2(SCRIPTS / name, repo / "scripts" / name)
            with (repo / "scripts/release-lib.sh").open("a") as library:
                library.write('\nfetch_master() { :; } # fixture has a local origin/master\n')
            (repo / "Cargo.toml").write_text('[workspace.package]\nversion = "0.2.3"\n')
            (repo / "CHANGELOG.md").write_text('## [Unreleased]\n\n## [0.2.3] - 2026-10-28\n')
            golden = repo / "tests/golden/e2e-baseline.libm-matched.portable.json"
            golden.parent.mkdir(parents=True)
            golden.write_text(json.dumps({"identical": ["fixture"]}))

            def git(*args):
                return subprocess.check_output(["git", "-c", "tag.gpgSign=false", "-c", "commit.gpgSign=false",
                                                "-c", "core.hooksPath=/dev/null", *args], cwd=repo, text=True,
                                               stderr=subprocess.STDOUT).strip()

            git("init", "-q")
            git("add", ".")
            author = ("-c", "user.name=Test", "-c", "user.email=test@example.invalid")
            git(*author, "commit", "-qm", "Published baseline")
            git("tag", "v0.2.3")
            (repo / "api-change").write_text("breaking API change\n")
            git("add", ".")
            git(*author, "commit", "-qm", "Breaking API (#1)")
            if rc:
                git("tag", "v1.0.0-rc.1")
            git("update-ref", "refs/remotes/origin/master", "HEAD")

            gh = binaries / "gh"
            gh.write_text('''#!/usr/bin/env python3
import json, sys
args = sys.argv[1:]
if args[:2] == ['issue', 'list']:
    print('') # gh applies the command's --jq before printing
elif args[:2] == ['release', 'list']:
    print('[]')
elif args[:2] == ['run', 'list']:
    print(json.dumps([{'status': 'completed', 'conclusion': 'success',
                      'url': 'https://example.test/run', 'event': 'push'}]))
elif args[:2] == ['api', 'graphql']:
    print(json.dumps({'data': {'repository': {'p1': {
        'title': 'Breaking API', 'url': 'https://example.test/pr/1',
        'body': '## Changelog entry\\n- **Breaking:** Changed API.',
        'labels': {'nodes': [{'name': 'breaking'}]}
    }}}}))
elif args[0] == 'api' and '/releases/tags/' in args[1]:
    sys.exit(1)
else:
    sys.exit(2)
''')
            cargo = binaries / "cargo"
            cargo.write_text('''#!/usr/bin/env python3
import os, sys
if sys.argv[1:] == ['semver-checks', '--version']:
    print('cargo-semver-checks 0.51.0')
    sys.exit(0)
assert os.environ['CARGO_TERM_COLOR'] == 'never'
for i in range(80):
    print(f'checker log line {i}')
package = sys.argv[sys.argv.index('-p') + 1]
sys.exit(101 if os.environ['TOOL_ERROR'] == 'true' else (100 if package == 'graphviz-pure' else 0))
''')
            curl = binaries / "curl"
            curl.write_text('#!/usr/bin/env python3\nprint(\'{"vers":"0.2.3","yanked":false}\')\n')
            for binary in (gh, cargo, curl):
                binary.chmod(0o755)
            env = dict(os.environ, PATH=f"{binaries}:{os.environ['PATH']}",
                       TOOL_ERROR=str(tool_error).lower(), VERSION_INPUT=version,
                       CARGO_TERM_COLOR="always", PYTHONDONTWRITEBYTECODE="1")
            for name in ("BASELINE_VERSION", "BREAKING", "RELEASE_TARGET", "SEMVER_MANIFEST", "GIT_DIR"):
                env.pop(name, None)
            result = subprocess.run(["bash", "scripts/release-check.sh"], cwd=repo, env=env,
                                    capture_output=True, text=True, check=False)
            return result

    def test_direct_one_point_zero_accepts_larger_bump(self):
        result = self.run_readiness("1.0.0")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("minimum 0.3.0", result.stdout)
        self.assertIn("requires a minor version bump", result.stdout)
        self.assertNotIn("checker log line", result.stdout)

    def test_explicit_patch_cannot_replace_required_minor(self):
        result = self.run_readiness("0.2.4")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("minimum is 0.3.0", result.stdout)

    def test_explicit_rc_then_final_retains_pre_rc_breaking_labels(self):
        result = self.run_readiness("1.0.0-rc.1")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        result = self.run_readiness("1.0.0", rc=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("since v1.0.0-rc.1", result.stdout)
        self.assertIn("0 with one", result.stdout) # no new PRs after rc; API labels still count

    def test_failure_shows_only_log_tail_without_color(self):
        result = self.run_readiness("1.0.0", tool_error=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checker log line 79", result.stdout)
        self.assertNotIn("checker log line 0\n", result.stdout)
        self.assertNotIn("\x1b", result.stdout)


if __name__ == "__main__":
    unittest.main()
