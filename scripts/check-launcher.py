#!/usr/bin/env python3
"""Exercise a compiled baseline launcher with controlled child processes (Unix).
Usage: scripts/check-launcher.py LAUNCHER
"""
import pathlib
import shutil
import signal
import subprocess
import sys
import tempfile

with tempfile.TemporaryDirectory(prefix="graphviz launcher ") as tmp:
    root = pathlib.Path(tmp)
    launcher = root / "graphviz-pure"
    shutil.copy2(sys.argv[1], launcher)
    children = [root / v / "graphviz-pure" for v in ("v3", "compat")]
    for child in children:
        child.parent.mkdir()
    for child in children:
        child.write_text('#!/bin/sh\ncat\nprintf "%s\\n" "$@"\nprintf diagnostic >&2\nexit 37\n')
        child.chmod(0o755)
    result = subprocess.run([str(launcher), "a b", "; echo injected", "--"], input=b"stdin\n", capture_output=True)
    assert (result.returncode, result.stdout, result.stderr) == (37, b"stdin\na b\n; echo injected\n--\n", b"diagnostic"), result
    alias_dir = root / "aliases"
    alias_dir.mkdir()
    alias = alias_dir / "neato"
    alias.symlink_to(launcher)
    result = subprocess.run([str(alias), "alias argument"], input=b"alias stdin\n", capture_output=True)
    assert (result.returncode, result.stdout, result.stderr) == (37, b"alias stdin\nalias argument\n", b"diagnostic"), result
    for child in children:
        child.write_text('#!/bin/sh\nkill -TERM $$\n')
    result = subprocess.run([str(launcher)])
    assert result.returncode == -signal.SIGTERM, result
    for child in children:
        child.unlink()
    result = subprocess.run([str(launcher)], capture_output=True)
    assert result.returncode == 1 and b"install the launcher together" in result.stderr, result
print("launcher: arguments, streams, exit status, signal and missing binary pass")
