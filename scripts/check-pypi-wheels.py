#!/usr/bin/env python3
"""Checks graphviz-pure VERSION on PyPI against the release's archives: one wheel per archive
(scripts/release-wheel.py's platform tags), each holding the archive's binary byte for byte,
and PyPI's sha256 matching the downloaded file. DIST holds the archives, already verified
against their attestations. Prints one summary line; exits 1 on any difference.

Usage: scripts/check-pypi-wheels.py VERSION DIST
PYPI_URL (default https://pypi.org) names another index with PyPI's JSON API, such as TestPyPI.
While PyPI has no graphviz-pure project at all, the channel is not set up yet (release.yml's pypi
job publishes nothing until its trusted publisher exists): that passes with a "skipped" line,
unless PYPI_REQUIRED=1 (release.yml's verify job, which checks only a channel that published).
"""
import hashlib
import io
import json
import os
import re
import sys
import tarfile
import urllib.error
import urllib.request
import zipfile

version, dist = sys.argv[1:3]
ua = {"User-Agent": "graphviz-pure release check (https://github.com/graphviz-pure/graphviz-pure)"}


def get(url):
    with urllib.request.urlopen(urllib.request.Request(url, headers=ua), timeout=120) as r:
        return r.read()


m = re.fullmatch(r"(\d+\.\d+\.\d+)(?:-(alpha|beta|rc)\.?(\d+))?", version)
if not m:
    sys.exit(f"{version} has no PEP 440 form")
pyversion = m.group(1) + ({"alpha": "a", "beta": "b", "rc": "rc"}[m.group(2)] + m.group(3) if m.group(2) else "")
platforms = {
    "x86_64-unknown-linux-musl": "manylinux_2_17_x86_64.manylinux2014_x86_64.musllinux_1_1_x86_64",
    "aarch64-unknown-linux-musl": "manylinux_2_17_aarch64.manylinux2014_aarch64.musllinux_1_1_aarch64",
    "x86_64-apple-darwin": "macosx_10_12_x86_64",
    "aarch64-apple-darwin": "macosx_11_0_arm64",
    "x86_64-pc-windows-msvc": "win_amd64",
    "aarch64-pc-windows-msvc": "win_arm64",
}
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
for target, tag in platforms.items():
    ext = ".zip" if "windows" in target else ".tar.gz"
    name = f"graphviz-pure-{version}-{target}"
    exe = "graphviz-pure.exe" if "windows" in target else "graphviz-pure"
    wheel = f"graphviz_pure-{pyversion}-py3-none-{tag}.whl"
    if wheel not in files:
        bad.append(f"no {wheel}")
        continue
    data = get(files[wheel]["url"])
    if hashlib.sha256(data).hexdigest() != files[wheel]["digests"]["sha256"]:
        bad.append(f"{wheel}: download differs from PyPI's sha256")
        continue
    archive = os.path.join(dist, name + ext)
    try:
        if ext == ".zip":
            want = zipfile.ZipFile(archive).read(f"{name}/{exe}")
        else:
            want = tarfile.open(archive).extractfile(f"{name}/{exe}").read()
        got = zipfile.ZipFile(io.BytesIO(data)).read(f"graphviz_pure-{pyversion}.data/scripts/{exe}")
    except (OSError, KeyError, tarfile.TarError, zipfile.BadZipFile) as e:
        bad.append(f"{wheel}: cannot compare with {name}{ext} ({e})")
        continue
    if got != want:
        bad.append(f"{wheel}: its binary is not {name}{ext}'s")
extra = sorted(set(files) - {f"graphviz_pure-{pyversion}-py3-none-{t}.whl" for t in platforms.values()})
if extra:
    bad.append("unexpected files: " + " ".join(extra))
if bad:
    sys.exit("; ".join(bad))
print(f"graphviz-pure {pyversion}: {len(platforms)} wheels, each holding its attested archive's binary")
