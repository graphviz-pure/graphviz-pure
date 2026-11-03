import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent


def make_fake(tmp: Path, name: str, code: int, stdout: bytes = b"", stderr: bytes = b""):
    wrapper = tmp / f"{name}.py"
    wrapper.write_text(
        f"#!/usr/bin/env python3\n"
        f"import sys\n"
        f"sys.stdout.buffer.write({stdout!r})\n"
        f"sys.stderr.buffer.write({stderr!r})\n"
        f"sys.exit({code})\n"
    )
    wrapper.chmod(0o755)
    p = tmp / name
    p.write_text(f"#!/bin/sh\n{wrapper} \"$@\"\n")
    p.chmod(0o755)
    return p


def run_check(c_bin: Path, p_bin: Path, gv_in: str):
    with tempfile.TemporaryDirectory() as d:
        d = Path(d)
        case = d / "t.gv"
        case.write_text(gv_in)
        env = os.environ.copy()
        env["C_DOT"] = str(c_bin)
        env["PORT_BIN"] = str(p_bin)
        env["PORT_DEBUG_BIN"] = str(p_bin)
        env["GVBINDIR"] = "/tmp"
        out = subprocess.run(
            [sys.executable, "-I", str(ROOT / "scripts/fuzz-run.py"), "check", "--no-debug", str(case)],
            env=env,
            cwd=ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        return out.stdout.decode(), out.stderr.decode()


class FuzzRunStderrTest(unittest.TestCase):
    def test_exit_diff_different_nonzero(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            c = make_fake(tmp, "c", 1, b"<svg>c</svg>", b"err1")
            p = make_fake(tmp, "p", 2, b"<svg>p</svg>", b"err2")
            out, _ = run_check(c, p, "digraph{a->b}")
            self.assertTrue(out.strip().startswith("exit_diff"), out)

    def test_exit_diff_nonzero_to_zero(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            c = make_fake(tmp, "c", 1, b"<svg>c</svg>", b"")
            p = make_fake(tmp, "p", 0, b"<svg>p</svg>", b"")
            out, _ = run_check(c, p, "digraph{a->b}")
            self.assertTrue(out.strip().startswith("exit_diff"), out)

    def test_same_exit_stderr_diff(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            c = make_fake(tmp, "c", 0, b"<svg>same</svg>", b"warn A")
            p = make_fake(tmp, "p", 0, b"<svg>same</svg>", b"warn B")
            out, _ = run_check(c, p, "digraph{a->b}")
            self.assertTrue(out.strip().startswith("same"), out)
            self.assertIn("stderr_diff", out)

    def test_svg_diff_stderr_diff(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            c = make_fake(tmp, "c", 0, b"<svg>a</svg>", b"x")
            p = make_fake(tmp, "p", 0, b"<svg>b</svg>", b"y")
            out, _ = run_check(c, p, "digraph{a->b}")
            self.assertTrue(out.strip().startswith("svg_diff"), out)
            self.assertIn("stderr_diff", out)


class FuzzRunStderrDetailsTest(unittest.TestCase):
    def test_details(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location('fuzzrun', ROOT / 'scripts/fuzz-run.py')
        m = importlib.util.module_from_spec(spec)
        sys.modules['fuzzrun'] = m
        spec.loader.exec_module(m)
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            c = make_fake(tmp, "c", 0, b"<svg>same</svg>", b"warn A")
            p = make_fake(tmp, "p", 0, b"<svg>same</svg>", b"warn B")
            case = tmp / "t.gv"
            case.write_text("digraph{a->b}")
            m.C_BIN = str(c)
            m.REL = str(p)
            m.DBG = str(p)
            m.C_ENV = dict(os.environ, GVBINDIR="/tmp")
            oc, ddet = m.classify(case, debug=False)
            self.assertEqual(oc, 'same')
            self.assertTrue(ddet.get('stderr_diff'))

            # svg diff stderr
            c2 = make_fake(tmp, "c2", 0, b"<svg>a</svg>", b"x")
            p2 = make_fake(tmp, "p2", 0, b"<svg>b</svg>", b"y")
            case2 = tmp / "t2.gv"
            case2.write_text("digraph{a->b}")
            m.C_BIN = str(c2)
            m.REL = str(p2)
            oc2, ddet2 = m.classify(case2, debug=False)
            self.assertEqual(oc2, 'svg_diff')
            self.assertTrue(ddet2.get('stderr_diff'))

            # exit diff different nonzero
            c3 = make_fake(tmp, "c3", 1, b"<svg>c</svg>", b"err1")
            p3 = make_fake(tmp, "p3", 2, b"<svg>p</svg>", b"err2")
            case3 = tmp / "t3.gv"
            case3.write_text("digraph{a->b}")
            m.C_BIN = str(c3)
            m.REL = str(p3)
            oc3, ddet3 = m.classify(case3, debug=False)
            self.assertEqual(oc3, 'exit_diff')

            # exit diff nonzero to zero
            c4 = make_fake(tmp, "c4", 1, b"<svg>c</svg>", b"")
            p4 = make_fake(tmp, "p4", 0, b"<svg>p</svg>", b"")
            case4 = tmp / "t4.gv"
            case4.write_text("digraph{a->b}")
            m.C_BIN = str(c4)
            m.REL = str(p4)
            oc4, ddet4 = m.classify(case4, debug=False)
            self.assertEqual(oc4, 'exit_diff')
