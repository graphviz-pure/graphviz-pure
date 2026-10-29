#!/usr/bin/env python3
"""Gate a native release PGO binary against a plain build over the whole SVG corpus.

Release x86-64 jobs reuse their already-built compat binary. ARM64 builds a plain
reference. CI uses --build-reference to create the compat reference too.
"""
import argparse
import json
import os
import pathlib
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--target", required=True)
parser.add_argument("--pgo-dir", default="target")
parser.add_argument("--plain-dir")
parser.add_argument("--build-reference", action="store_true")
parser.add_argument("--report", type=pathlib.Path, required=True)
parser.add_argument("--cargo-wrapper")
args = parser.parse_args()
os.chdir(ROOT)
x86 = args.target.startswith("x86_64")
plain_dir = pathlib.Path(args.plain_dir or ("target-compat" if x86 else "target-plain")).resolve()
pgo_dir = pathlib.Path(args.pgo_dir).resolve()
if plain_dir == pgo_dir:
    parser.error("plain and PGO builds need separate target directories")
exe = "graphviz-pure.exe" if "windows" in args.target else "graphviz-pure"
optimised = pgo_dir / args.target / "release" / exe
reference = plain_dir / args.target / "release" / exe
if not optimised.is_file():
    sys.exit(f"missing PGO binary: {optimised}")
plain_seconds = 0.0
if args.build_reference or not x86:
    # The reference has no PGO or x86-v3 flag. Windows keeps the shipped static CRT.
    flags = ["-C", "target-feature=+crt-static"] if "windows" in args.target else []
    env = dict(os.environ, CARGO_TARGET_DIR=str(plain_dir), CARGO_ENCODED_RUSTFLAGS="\x1f".join(flags))
    env.pop("RUSTFLAGS", None)
    cargo = ([args.cargo_wrapper] if args.cargo_wrapper else []) + ["cargo"]
    before = time.monotonic()
    subprocess.run([*cargo, "build", "--release", "--locked", "--target", args.target,
                    "-p", "graphviz-pure-cli", "--bin", "graphviz-pure"], env=env, check=True)
    plain_seconds = time.monotonic() - before
if not reference.is_file():
    sys.exit(f"missing plain reference: {reference}")
subprocess.run([sys.executable, "scripts/check-cpu-builds.py", str(reference), str(optimised),
                "--report", str(args.report), "--label", args.target], check=True)
report = json.loads(args.report.read_text())
report["plain_build_seconds"] = plain_seconds
# x86 compat was already required by the release, regardless of this new gate.
report["additional_release_seconds"] = report["elapsed_seconds"] + (0.0 if x86 else plain_seconds)
args.report.write_text(json.dumps(report, indent=2) + "\n")
text = (f"{args.target}: plain build {plain_seconds:.1f}s; added release validation "
        f"{report['additional_release_seconds']:.1f}s "
        f"({'reuse existing compat' if x86 else 'plain build + corpus comparison'})")
print(text, flush=True)
if os.environ.get("GITHUB_STEP_SUMMARY"):
    with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as summary:
        summary.write(f"\n{text}.\n")
