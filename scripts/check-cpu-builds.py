#!/usr/bin/env python3
"""Compare plain/optimised SVG bytes for every tests/**/*.gv input (layout= selects the engine).

Also used for v1/v3 CPU variants. Invalid inputs must fail identically too.
"""
import argparse
import json
import os
import pathlib
import subprocess
import sys
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("reference")
parser.add_argument("optimised")
parser.add_argument("--report", type=pathlib.Path, help="write counts and timings as JSON")
parser.add_argument("--label", default="CPU build comparison")
parser.add_argument("--corpus", type=pathlib.Path, default=pathlib.Path("tests"))
args = parser.parse_args()

identical = 0
bad = []
seconds = [0.0, 0.0]
start = time.monotonic()
inputs = sorted(args.corpus.rglob("*.gv"))
if not inputs:
    parser.error("the corpus contains no .gv inputs")
for path in inputs:
    outputs = []
    for index, binary in enumerate([args.reference, args.optimised]):
        before = time.monotonic()
        try:
            result = subprocess.run([binary, "-Tsvg", str(path)], capture_output=True, timeout=600)
        except subprocess.TimeoutExpired:
            sys.exit(f"{path}: timed out with {binary}")
        seconds[index] += time.monotonic() - before
        if result.returncode < 0:
            sys.exit(f"{path}: {binary} terminated by signal {-result.returncode}")
        if result.returncode >= 0x80000000:
            # Windows exceptions (access violation, stack overflow, ...) are NTSTATUS
            # exit codes, not negative POSIX signals. Equal crashes must still fail.
            sys.exit(f"{path}: {binary} terminated with Windows status 0x{result.returncode:08x}")
        outputs.append((result.returncode, result.stdout, result.stderr))
    if outputs[0] != outputs[1]:
        bad.append(str(path))
    else:
        identical += 1
        if identical % 100 == 0:
            print(f"{identical} inputs identical so far", flush=True)
elapsed = time.monotonic() - start
report = {"label": args.label, "identical": identical, "different": bad,
          "elapsed_seconds": elapsed, "reference_seconds": seconds[0], "optimised_seconds": seconds[1]}
if args.report:
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n")
print(f"{identical} inputs identical (SVG bytes, diagnostics and exit status); {len(bad)} differ", flush=True)
print(f"{args.label}: corpus comparison {elapsed:.1f}s "
      f"(reference {seconds[0]:.1f}s, optimised {seconds[1]:.1f}s)", flush=True)
if os.environ.get("GITHUB_STEP_SUMMARY"):
    with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as summary:
        summary.write(f"\n{args.label}: {identical} identical, {len(bad)} different; "
                      f"full-corpus comparison **{elapsed:.1f}s** "
                      f"(reference {seconds[0]:.1f}s, optimised {seconds[1]:.1f}s).\n")
if bad:
    sys.exit("\n".join(bad))
