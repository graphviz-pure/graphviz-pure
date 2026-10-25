#!/usr/bin/env python3
"""Repackages one release archive as a Python wheel: graphviz_pure-VERSION-py3-none-PLATFORM.whl
in OUT, whose scripts directory holds the archive's binary byte for byte, so `pip install
graphviz-pure` (or `uvx graphviz-pure`) puts the attested `graphviz-pure` command on the
environment's PATH, the way ruff and uv ship. Nothing is compiled and nothing is imported: the
wheel works with any Python 3.

The wheel is deterministic (sorted entries, fixed timestamps), so a rebuilt wheel is identical and
PyPI accepts a re-run of the upload. The archive's licence files go to the wheel's dist-info
(PEP 639), and packaging/pypi/README.md is its description.

Usage: scripts/release-wheel.py ARCHIVE OUT
  ARCHIVE  graphviz-pure-VERSION-TARGET.{tar.gz,zip} (scripts/release-package.py)
"""
import base64
import hashlib
import io
import os
import re
import sys
import tarfile
import zipfile

# Each release target's wheel platform tags. The Linux binaries are static (musl), so they run on
# glibc and musl systems alike; the macOS ones need the binaries' minimum OS versions (LC_BUILD_VERSION).
PLATFORMS = {
    "x86_64-unknown-linux-musl": "manylinux_2_17_x86_64.manylinux2014_x86_64.musllinux_1_1_x86_64",
    "aarch64-unknown-linux-musl": "manylinux_2_17_aarch64.manylinux2014_aarch64.musllinux_1_1_aarch64",
    "x86_64-apple-darwin": "macosx_10_12_x86_64",
    "aarch64-apple-darwin": "macosx_11_0_arm64",
    "x86_64-pc-windows-msvc": "win_amd64",
    "aarch64-pc-windows-msvc": "win_arm64",
}
SUMMARY = "Pure-Rust port of Graphviz 16.1.0: the dot command line, same output on every platform (not affiliated with Graphviz)"
LICENSE = "EPL-2.0 AND OFL-1.1 AND MIT"


def pep440(version):
    """The PEP 440 form of a release version: 0.3.0-rc.1 is 0.3.0rc1 (alpha, beta and rc only)."""
    m = re.fullmatch(r"(\d+\.\d+\.\d+)(?:-(alpha|beta|rc)\.?(\d+))?", version)
    if not m:
        sys.exit(f"{version}: no PEP 440 form (X.Y.Z, or X.Y.Z-alpha.N, -beta.N, -rc.N)")
    base, pre, n = m.groups()
    return base + ({"alpha": "a", "beta": "b", "rc": "rc"}[pre] + n if pre else "")


archive, out = sys.argv[1:3]
m = re.fullmatch(r"graphviz-pure-(.+?)-([a-z0-9_]+-[a-z0-9_]+-[a-z0-9_]+(?:-[a-z0-9_]+)?)\.(tar\.gz|zip)", os.path.basename(archive))
if not m or m.group(2) not in PLATFORMS:
    sys.exit(f"{archive}: not a release archive of a target with a wheel ({', '.join(PLATFORMS)})")
version, target, _ = m.groups()
name = f"graphviz-pure-{version}-{target}"
exe = "graphviz-pure.exe" if "windows" in target else "graphviz-pure"
if archive.endswith(".zip"):
    z = zipfile.ZipFile(archive)
    files = {n[len(name) + 1:]: z.read(n) for n in z.namelist() if n.startswith(name + "/") and not n.endswith("/")}
else:
    t = tarfile.open(archive)
    files = {mem.name[len(name) + 1:]: t.extractfile(mem).read() for mem in t.getmembers() if mem.isfile() and mem.name.startswith(name + "/")}
licences = sorted(f for f in files if f in ("LICENSE", "NOTICE", "THIRD-PARTY-LICENSES.md") or f.startswith("LICENSES/"))
assert exe in files and "LICENSE" in files and len(licences) >= 6, f"{archive}: unexpected contents {sorted(files)}"

pyversion = pep440(version)
dist = f"graphviz_pure-{pyversion}"
tags = [f"py3-none-{p}" for p in PLATFORMS[target].split(".")]
readme = open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "packaging", "pypi", "README.md"), "rb").read()
metadata = "".join(
    [
        "Metadata-Version: 2.4\n",
        "Name: graphviz-pure\n",
        f"Version: {pyversion}\n",
        f"Summary: {SUMMARY}\n",
        f"License-Expression: {LICENSE}\n",
    ]
    + [f"License-File: {f}\n" for f in licences]
    + [
        "Project-URL: Homepage, https://graphviz-pure.github.io/\n",
        "Project-URL: Source, https://github.com/graphviz-pure/graphviz-pure\n",
        "Project-URL: Issues, https://github.com/graphviz-pure/graphviz-pure/issues\n",
        "Project-URL: Changelog, https://github.com/graphviz-pure/graphviz-pure/blob/master/CHANGELOG.md\n",
        "Keywords: graphviz,dot,graph,diagram,svg,layout\n",
        "Classifier: Development Status :: 4 - Beta\n",
        "Classifier: Environment :: Console\n",
        "Classifier: Intended Audience :: Developers\n",
        "Classifier: Intended Audience :: Science/Research\n",
        "Classifier: Programming Language :: Rust\n",
        "Classifier: Topic :: Multimedia :: Graphics :: Presentation\n",
        "Classifier: Topic :: Scientific/Engineering :: Visualization\n",
        "Description-Content-Type: text/markdown\n",
        "\n",
    ]
).encode() + readme
wheel = "".join(
    ["Wheel-Version: 1.0\n", "Generator: graphviz-pure scripts/release-wheel.py\n", "Root-Is-Purelib: false\n"]
    + [f"Tag: {t}\n" for t in tags]
).encode()

entries = [(f"{dist}.data/scripts/{exe}", files[exe], 0o755)]
entries += [(f"{dist}.dist-info/licenses/{f}", files[f], 0o644) for f in licences]
entries += [(f"{dist}.dist-info/METADATA", metadata, 0o644), (f"{dist}.dist-info/WHEEL", wheel, 0o644)]


def record_line(path, data):
    digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode()
    return f"{path},sha256={digest},{len(data)}\n"


record_path = f"{dist}.dist-info/RECORD"
record = ("".join(record_line(p, d) for p, d, _ in entries) + f"{record_path},,\n").encode()
entries.append((record_path, record, 0o644))

os.makedirs(out, exist_ok=True)
path = os.path.join(out, f"{dist}-py3-none-{PLATFORMS[target]}.whl")
buf = io.BytesIO()
with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
    # The scripts first and RECORD last, as wheel builders write them; fixed times and modes.
    for p, data, mode in entries:
        info = zipfile.ZipInfo(p, date_time=(1980, 1, 1, 0, 0, 0))
        info.external_attr = (mode | 0o100000) << 16
        info.compress_type = zipfile.ZIP_DEFLATED
        z.writestr(info, data)
with open(path, "wb") as f:
    f.write(buf.getvalue())
print(path)
