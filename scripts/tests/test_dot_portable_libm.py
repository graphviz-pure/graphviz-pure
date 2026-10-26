"""Tests for dot-portable-libm.sh's GVBINDIR guard (run with `python3 -m unittest`): the wrapper
must refuse to run C with a default or full-installation plugin configuration, because then a
pango textlayout plugin measures C's text instead of the built-in font metric tables and every
comparison silently differs by millipoints (hidden #455)."""
import pathlib
import subprocess
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "dot-portable-libm.sh"
ENV = {"PATH": "/usr/bin:/bin:/opt/homebrew/bin", "HOME": str(pathlib.Path.home())}


class GvbindirGuardTest(unittest.TestCase):
    def test_refuses_to_run_without_gvbindir(self):
        p = subprocess.run([str(SCRIPT)], capture_output=True, text=True, env=ENV)
        self.assertEqual(p.returncode, 2)
        self.assertIn("GVBINDIR", p.stderr)
        self.assertIn("pango-free", p.stderr)

    def test_refuses_a_directory_without_a_plugin_config(self):
        with tempfile.TemporaryDirectory() as d:
            p = subprocess.run([str(SCRIPT)], capture_output=True, text=True,
                               env=dict(ENV, GVBINDIR=d))
        self.assertEqual(p.returncode, 2)
        self.assertIn("pango-free", p.stderr)

    def test_refuses_a_config_with_a_textlayout_plugin(self):
        # A full installation's plugin directory: its config names a textlayout (pango) plugin.
        with tempfile.TemporaryDirectory() as d:
            pathlib.Path(d, "config8").write_text(
                "libgvplugin_pango.8.dylib textlayout textlayout pango\n")
            p = subprocess.run([str(SCRIPT)], capture_output=True, text=True,
                               env=dict(ENV, GVBINDIR=d))
        self.assertEqual(p.returncode, 2)
        self.assertIn("pango-free", p.stderr)


if __name__ == "__main__":
    unittest.main()
