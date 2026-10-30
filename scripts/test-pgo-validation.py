#!/usr/bin/env python3
"""Exercise the release corpus gate's failure paths without building a product binary."""
import json
import pathlib
import runpy
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[1]
CHECK_PROFILE = runpy.run_path(str(ROOT / "scripts/build-pgo.py"))["check_profile"]


class ProfileGuard(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.profile = pathlib.Path(self.temp.name) / "merged.profdata"

    def test_empty_and_small_profiles_fail_before_profile_use(self):
        for size in [0, 64 * 1024 - 1]:
            self.profile.write_bytes(b"\0" * size)
            with self.assertRaisesRegex(RuntimeError, "smaller than 64 KiB"):
                CHECK_PROFILE(self.profile, "llvm-profdata")

    def test_large_but_untrained_profiles_fail(self):
        self.profile.write_bytes(b"\0" * (128 * 1024))
        for functions, maximum, total in [(0, 0, 0), (10000, 0, 0), (10000, 100, 999999)]:
            summary = (f"Total functions: {functions}\nMaximum function count: {maximum}\n"
                       f"Total count: {total}\n")
            with mock.patch("subprocess.check_output", return_value=summary):
                with self.assertRaisesRegex(RuntimeError, "too little training"):
                    CHECK_PROFILE(self.profile, "llvm-profdata")

    def test_meaningful_profile_passes_and_records_counts(self):
        self.profile.write_bytes(b"\0" * (128 * 1024))
        summary = "Total functions: 2000\nMaximum function count: 50000\nTotal count: 2000000\n"
        with mock.patch("subprocess.check_output", return_value=summary):
            counts = CHECK_PROFILE(self.profile, "llvm-profdata")
        self.assertEqual(counts["Total count"], 2000000)


class CorpusGate(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)
        self.corpus = self.root / "corpus"
        (self.corpus / "nested").mkdir(parents=True)
        (self.corpus / "a.gv").write_text("good")
        (self.corpus / "nested/b.gv").write_text("bad")
        (self.corpus / "ignored.txt").write_text("ignored")

    def binary(self, name, extra=""):
        path = self.root / name
        path.write_text(f"#!{sys.executable}\nimport pathlib,sys\n"
                        "text=pathlib.Path(sys.argv[-1]).read_text()\n"
                        "print(text)\n"
                        "print('diagnostic',file=sys.stderr)\n" + extra +
                        "sys.exit(2 if text=='bad' else 0)\n")
        path.chmod(0o755)
        return path

    def run_gate(self, left, right):
        return subprocess.run([sys.executable, str(ROOT / "scripts/check-cpu-builds.py"),
                               str(left), str(right), "--corpus", str(self.corpus),
                               "--report", str(self.root / "report.json")], capture_output=True,
                              text=True)

    def test_every_nested_input_and_identical_invalid_status(self):
        result = self.run_gate(self.binary("left"), self.binary("right"))
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads((self.root / "report.json").read_text())
        self.assertEqual(report["identical"], 2)
        self.assertEqual(report["different"], [])
        self.assertGreater(report["elapsed_seconds"], 0)

    def test_stdout_stderr_and_exit_mismatches_fail(self):
        for extra in ["print('different')\n", "print('different',file=sys.stderr)\n", "sys.exit(3)\n"]:
            with self.subTest(extra=extra):
                result = self.run_gate(self.binary("left"), self.binary("right", extra))
                self.assertNotEqual(result.returncode, 0)
                report = json.loads((self.root / "report.json").read_text())
                self.assertEqual(len(report["different"]), 2)

    def test_signal_fails_without_accepting_equal_crashes(self):
        crash = "import os,signal\nos.kill(os.getpid(),signal.SIGTERM)\n"
        result = self.run_gate(self.binary("left", crash), self.binary("right", crash))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("terminated by signal", result.stderr)

    def test_windows_exception_fails_without_accepting_equal_crashes(self):
        argv = ["check-cpu-builds.py", "left.exe", "right.exe", "--corpus", str(self.corpus)]
        crash = subprocess.CompletedProcess([], 0xC0000005, b"", b"")
        with mock.patch.object(sys, "argv", argv), mock.patch("subprocess.run", return_value=crash):
            with self.assertRaisesRegex(SystemExit, "Windows status 0xc0000005"):
                runpy.run_path(str(ROOT / "scripts/check-cpu-builds.py"))

    def test_empty_corpus_fails(self):
        for path in self.corpus.rglob("*.gv"):
            path.unlink()
        result = self.run_gate(self.binary("left"), self.binary("right"))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no .gv inputs", result.stderr)

    def test_held_out_sources_never_appear_in_training(self):
        def inputs(name):
            return {line.split()[0] for line in (ROOT / "scripts/bench/sets" / name).read_text().splitlines()
                    if line.strip() and not line.startswith("#")}
        held_out = inputs("pgo-held-out.tsv")
        self.assertEqual(len(held_out), 8)
        self.assertFalse(held_out & inputs("pgo.tsv"))


if __name__ == "__main__":
    unittest.main()
