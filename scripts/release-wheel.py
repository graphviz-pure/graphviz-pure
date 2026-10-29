#!/usr/bin/env python3
"""Repackages one release archive as a Python wheel: graphviz_pure-VERSION-py3-none-PLATFORM.whl
in OUT, whose scripts directory holds the archive binaries byte for byte, so `pip install
graphviz-pure` (or `uvx graphviz-pure`) puts the attested `graphviz-pure` command on the
environment's PATH, the way ruff and uv ship. Nothing is compiled and nothing is imported: the
wheel works with any Python 3.

The wheel is deterministic for a given Python and zlib: sorted entries, fixed timestamps and
Unix file attributes, and LF line endings in METADATA on every host. The archive's licence files go to the wheel's dist-info
(PEP 639), and packaging/pypi/README.md is its description.

Usage: scripts/release-wheel.py ARCHIVE OUT [--compat COMPAT_ARCHIVE]
  x86-64 requires the matching compat archive; its launcher comes from ARCHIVE.
  ARCHIVE  graphviz-pure-VERSION-TARGET.{tar.gz,zip} (scripts/release-package.py)
       scripts/release-wheel.py --pep440 VERSION
  prints VERSION's PEP 440 form, or fails if it has none (release.yml's check job runs it, so a
  version PyPI cannot take fails before anything is built)
"""
import base64
import hashlib
import io
import os
import re
import struct
import sys
import tarfile
import zipfile

# Each release target's wheel platform tags. The Linux binaries are static (musl), so they run on
# glibc and musl systems alike. The macOS tags carry the binary's minimum OS version, read from
# its Mach-O header (macosx_tag below); None here.
PLATFORMS = {
    "x86_64-unknown-linux-musl": "manylinux_2_17_x86_64.manylinux2014_x86_64.musllinux_1_1_x86_64",
    "aarch64-unknown-linux-musl": "manylinux_2_17_aarch64.manylinux2014_aarch64.musllinux_1_1_aarch64",
    "x86_64-apple-darwin": None,
    "aarch64-apple-darwin": None,
    "x86_64-pc-windows-msvc": "win_amd64",
    "aarch64-pc-windows-msvc": "win_arm64",
}
SUMMARY = "Pure-Rust port of Graphviz 16.1.0: the dot command line, same output on every platform (not affiliated with Graphviz)"
LICENSE = "EPL-2.0 AND OFL-1.1 AND MIT"


def pep440(version):
    """The PEP 440 form of a release version: 0.3.0-rc.1 is 0.3.0rc1 (alpha, beta and rc only)."""
    m = re.fullmatch(r"(\d+\.\d+\.\d+)(?:-(alpha|beta|rc)\.(\d+))?", version)
    if not m:
        sys.exit(f"{version}: no PEP 440 form (X.Y.Z, or X.Y.Z-alpha.N, -beta.N, -rc.N)")
    base, pre, n = m.groups()
    return base + ({"alpha": "a", "beta": "b", "rc": "rc"}[pre] + str(int(n)) if pre else "")


def macosx_tag(binary, arch):
    """macosx_X_Y_ARCH from the Mach-O's minimum OS version: LC_BUILD_VERSION (arm64) or
    LC_VERSION_MIN_MACOSX (x86_64), so the tag follows the toolchain's deployment target."""
    magic, _, _, _, ncmds, _, _ = struct.unpack_from("<7I", binary, 0)
    if magic != 0xFEEDFACF:
        sys.exit("not a 64-bit little-endian Mach-O")
    off = 32
    for _ in range(ncmds):
        cmd, size = struct.unpack_from("<2I", binary, off)
        if cmd in (0x32, 0x24):  # LC_BUILD_VERSION (platform, minos), LC_VERSION_MIN_MACOSX (version)
            if cmd == 0x32 and struct.unpack_from("<I", binary, off + 8)[0] != 1:
                sys.exit("LC_BUILD_VERSION is not for macOS")
            v = struct.unpack_from("<I", binary, off + (12 if cmd == 0x32 else 8))[0]
            major, minor = v >> 16, (v >> 8) & 0xFF
            # From macOS 11, pip matches the major version only.
            return f"macosx_{major}_{0 if major >= 11 else minor}_{arch}"
        off += size
    sys.exit("the Mach-O has no minimum OS version")


if sys.argv[1:2] == ["--pep440"] and len(sys.argv) == 3:
    print(pep440(sys.argv[2]))
    sys.exit(0)
if len(sys.argv) not in (3, 5):
    sys.exit(__doc__)
archive, out = sys.argv[1:3]
base = os.path.basename(archive)
match = [t for t in PLATFORMS if re.fullmatch(rf"graphviz-pure-.+-{re.escape(t)}\.(tar\.gz|zip)", base)]
if len(match) != 1:
    sys.exit(f"{archive}: not a release archive of a target with a wheel ({', '.join(PLATFORMS)})")
target = match[0]
if not target.startswith("x86_64") and len(sys.argv) != 3:
    sys.exit("--compat is only supported for x86-64 targets")
version = re.fullmatch(rf"graphviz-pure-(.+)-{re.escape(target)}\.(tar\.gz|zip)", base).group(1)
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
platform = PLATFORMS[target] or macosx_tag(files[exe], "arm64" if target.startswith("aarch64") else "x86_64")
tags = [f"py3-none-{p}" for p in platform.split(".")]
# LF whatever the checkout's line endings (Windows checkouts are CRLF), so every runner writes
# the same METADATA.
readme = open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "packaging", "pypi", "README.md"), "rb").read().replace(b"\r\n", b"\n")
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
if target.startswith("x86_64"):
    if len(sys.argv) != 5 or sys.argv[3] != "--compat":
        sys.exit("x86-64 wheels require --compat ARCHIVE (baseline binary)")
    compat_archive = sys.argv[4]
    expected = name + "-compat"
    if os.path.basename(compat_archive) != expected + (".zip" if "windows" in target else ".tar.gz"):
        sys.exit("compat archive must have the same version and target")
    if compat_archive.endswith(".zip"):
        with zipfile.ZipFile(compat_archive) as z:
            compat_binary = z.read(f"{expected}/{exe}")
    else:
        with tarfile.open(compat_archive) as t:
            compat_binary = t.extractfile(f"{expected}/{exe}").read()
    extension = ".exe" if "windows" in target else ""
    entries = [
        (f"{dist}.data/scripts/{exe}", files["graphviz-pure-launcher" + extension], 0o755),
        (f"{dist}.data/scripts/v3/{exe}", files[exe], 0o755),
        (f"{dist}.data/scripts/compat/{exe}", compat_binary, 0o755),
    ]
entries += [(f"{dist}.dist-info/licenses/{f}", files[f], 0o644) for f in licences]
entries += [(f"{dist}.dist-info/METADATA", metadata, 0o644), (f"{dist}.dist-info/WHEEL", wheel, 0o644)]


def record_line(path, data):
    digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode()
    return f"{path},sha256={digest},{len(data)}\n"


record_path = f"{dist}.dist-info/RECORD"
record = ("".join(record_line(p, d) for p, d, _ in entries) + f"{record_path},,\n").encode()
entries.append((record_path, record, 0o644))

os.makedirs(out, exist_ok=True)
path = os.path.join(out, f"{dist}-py3-none-{platform}.whl")
buf = io.BytesIO()
with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
    # The scripts first and RECORD last, as wheel builders write them; fixed times and modes.
    for p, data, mode in entries:
        info = zipfile.ZipInfo(p, date_time=(1980, 1, 1, 0, 0, 0))
        info.create_system = 3  # Unix, on every host: the modes below are Unix modes
        info.external_attr = (mode | 0o100000) << 16
        info.compress_type = zipfile.ZIP_DEFLATED
        z.writestr(info, data)
with open(path, "wb") as f:
    f.write(buf.getvalue())
print(path)
