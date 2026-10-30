#!/usr/bin/env python3
"""Compare v1/v3 SVG bytes for every tests/**/*.gv input (its layout= selects the engine).

Usage: scripts/check-cpu-builds.py V1_BINARY V3_BINARY
"""
import pathlib
import subprocess
import sys

if len(sys.argv) != 3:
    sys.exit(__doc__)

identical = 0
bad = []
for path in sorted(pathlib.Path("tests").rglob("*.gv")):
    outputs = []
    for binary in sys.argv[1:3]:
        try:
            result = subprocess.run([binary, "-Tsvg", str(path)], capture_output=True, timeout=600)
        except subprocess.TimeoutExpired:
            sys.exit(f"{path}: timed out with {binary}")
        # Invalid inputs must fail identically too, without hiding a crash.
        if result.returncode < 0:
            sys.exit(f"{path}: {binary} terminated by signal {-result.returncode}")
        outputs.append((result.returncode, result.stdout, result.stderr))
    if outputs[0] != outputs[1]:
        bad.append(str(path))
    else:
        identical += 1
        if identical % 100 == 0:
            print(f"{identical} inputs identical so far", flush=True)
print(f"{identical} inputs identical (SVG bytes, diagnostics and exit status); {len(bad)} differ", flush=True)
if bad:
    sys.exit("\n".join(bad))
