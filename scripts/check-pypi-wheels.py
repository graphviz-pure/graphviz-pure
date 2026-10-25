#!/usr/bin/env python3
"""Downloads graphviz-pure VERSION's files from PyPI into OUT and checks them: exactly one wheel
for each release platform (the tags of scripts/release-wheel.py), nothing else, none yanked, and
each download matching PyPI's sha256. scripts/release-verify.sh then checks each wheel's
build-provenance attestation, which covers the binary and the metadata. Prints one summary line;
exits 1 on any difference.

PYPI_URL (default https://pypi.org) names another index with PyPI's JSON API, such as TestPyPI.
While PyPI has no graphviz-pure project at all, the channel is not set up yet (release.yml's pypi
job publishes nothing until its trusted publisher exists): that passes with a "skipped" line,
unless PYPI_REQUIRED=1 (release.yml's verify job, which checks only a channel that published).

Usage: scripts/check-pypi-wheels.py VERSION OUT
"""
import hashlib
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request

if len(sys.argv) != 3:
    sys.exit(__doc__)
version, out = sys.argv[1:3]
here = os.path.dirname(os.path.abspath(__file__))
ua = {"User-Agent": "graphviz-pure release check (https://github.com/graphviz-pure/graphviz-pure)"}
# One wheel per release target: the fixed tags, and macOS's minimum-OS tags, which follow the build.
PLATFORMS = {
    "x86_64-unknown-linux-musl": r"manylinux_2_17_x86_64\.manylinux2014_x86_64\.musllinux_1_1_x86_64",
    "aarch64-unknown-linux-musl": r"manylinux_2_17_aarch64\.manylinux2014_aarch64\.musllinux_1_1_aarch64",
    "x86_64-apple-darwin": r"macosx_\d+_\d+_x86_64",
    "aarch64-apple-darwin": r"macosx_\d+_\d+_arm64",
    "x86_64-pc-windows-msvc": r"win_amd64",
    "aarch64-pc-windows-msvc": r"win_arm64",
}


def get(url):
    with urllib.request.urlopen(urllib.request.Request(url, headers=ua), timeout=120) as r:
        return r.read()


r = subprocess.run([sys.executable, "-I", os.path.join(here, "release-wheel.py"), "--pep440", version], capture_output=True, text=True)
if r.returncode:
    sys.exit(r.stderr.strip())
pyversion = r.stdout.strip()
index = os.environ.get("PYPI_URL", "https://pypi.org").rstrip("/")
if os.environ.get("PYPI_REQUIRED") != "1":
    try:
        get(f"{index}/pypi/graphviz-pure/json")
    except urllib.error.HTTPError as e:
        if e.code == 404:
            print("skipped: not set up yet (PyPI has no graphviz-pure project; docs/releasing.md)")
            sys.exit(0)
        raise
try:
    release = json.loads(get(f"{index}/pypi/graphviz-pure/{pyversion}/json"))
except Exception as e:
    sys.exit(f"graphviz-pure {pyversion} is not on PyPI ({e})")
files = {u["filename"]: u for u in release["urls"]}
bad = []
if release.get("info", {}).get("yanked"):
    bad.append(f"{pyversion} is yanked")
found = set()
os.makedirs(out, exist_ok=True)
for target, tag in PLATFORMS.items():
    names = [f for f in files if re.fullmatch(rf"graphviz_pure-{re.escape(pyversion)}-py3-none-{tag}\.whl", f)]
    if len(names) != 1:
        bad.append(f"{target}: {len(names)} wheels")
        continue
    name = names[0]
    found.add(name)
    if files[name].get("yanked"):
        bad.append(f"{name} is yanked")
    data = get(files[name]["url"])
    if hashlib.sha256(data).hexdigest() != files[name]["digests"]["sha256"]:
        bad.append(f"{name}: download differs from PyPI's sha256")
        continue
    with open(os.path.join(out, name), "wb") as f:
        f.write(data)
extra = sorted(set(files) - found)
if extra:
    bad.append("unexpected files: " + " ".join(extra))
if bad:
    sys.exit("; ".join(bad))
print(f"graphviz-pure {pyversion}: {len(found)} wheels")
