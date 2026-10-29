#!/usr/bin/env python3
"""Release smoke test: the binary must reproduce, byte for byte, every gallery sample
(site/src/data/samples.json) as committed in site/public/samples/NAME/rust.svg. Every build
computes with the same math library, so every platform writes those bytes. A release that does
not is not published.

Usage: scripts/release-smoke.py BINARY
"""
import argparse
import json
import os
import subprocess
import sys

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("binary")
parser.add_argument("--compare", action="append", default=[], help="direct binary whose version/error bytes must match")
args = parser.parse_args()
binary = args.binary
for flags in (["-V"], ["-K__launcher_error__"]):
    actual = subprocess.run([binary, *flags], input=b"digraph { a -> b }", capture_output=True)
    for direct in args.compare:
        expected = subprocess.run([direct, *flags], input=b"digraph { a -> b }", capture_output=True)
        if (actual.returncode, actual.stdout, actual.stderr) != (expected.returncode, expected.stdout, expected.stderr):
            sys.exit(f"launcher/direct output differs for {flags}: {actual.stderr!r} != {expected.stderr!r}")
if args.compare:
    print("launcher/direct version and error bytes identical")
root = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
samples = json.load(open(os.path.join(root, "site/src/data/samples.json")))
print(subprocess.run([binary, "-V"], capture_output=True, text=True).stderr.strip())
bad = []
for s in samples:
    d = os.path.join(root, "site/public/samples", s["name"])
    r = subprocess.run([binary, f"-K{s['engine']}", "-Tsvg", os.path.join(d, s["wasm_input"])], capture_output=True)
    want = open(os.path.join(d, "rust.svg"), "rb").read()
    if r.stdout != want:
        bad.append(s["name"])
print(f"{len(samples) - len(bad)}/{len(samples)} samples byte-identical")
if bad:
    sys.exit("differ: " + ", ".join(bad))
