#!/usr/bin/env python3
"""Packages one release binary: graphviz-pure-VERSION-TARGET.{tar.gz,zip} in OUT, holding the
binary (graphviz-pure.wasm for the WASI build), LICENSE, NOTICE, THIRD-PARTY-LICENSES.md (the licences of the Rust crates compiled into
it), README.md and LICENSES/ (the MIT licence of the CORE-MATH pow port and the OFL licences of
the bundled fonts) in a directory of the same name.

The archive is deterministic: sorted entries, fixed permissions and the commit's timestamp
(SOURCE_DATE_EPOCH), so the same build always gives the same archive bytes.

Usage: scripts/release-package.py TARGET VERSION OUT [--compat]
The compat build is read from target-compat; default builds from target.
Default x86-64 archives also carry target-launcher/graphviz-pure-launcher[.exe].
"""
import gzip
import io
import os
import sys
import tarfile
import zipfile

target, version, out = sys.argv[1:4]
compat = "--compat" in sys.argv[4:]
if compat and not target.startswith("x86_64"):
    sys.exit("--compat is only supported for x86-64 targets")
suffix = "-compat" if compat else ""
build_dir = "target-compat" if compat else "target"
root = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
windows = "windows" in target
# The WASI build (wasm32-wasip1) is a module for a WebAssembly runtime, not an executable.
exe = "graphviz-pure.exe" if windows else "graphviz-pure.wasm" if target.startswith("wasm32-") else "graphviz-pure"
name = f"graphviz-pure-{version}-{target}{suffix}"
epoch = int(os.environ.get("SOURCE_DATE_EPOCH", "0"))
fonts = os.path.join(root, "crates", "graphviz-pure-raster", "fonts")
files = [
    (os.path.join(root, build_dir, target, "release", exe), exe, 0o755),
    (os.path.join(root, "LICENSE"), "LICENSE", 0o644),
    (os.path.join(root, "NOTICE"), "NOTICE", 0o644),
    (os.path.join(root, "THIRD-PARTY-LICENSES.md"), "THIRD-PARTY-LICENSES.md", 0o644),
    (os.path.join(root, "README.md"), "README.md", 0o644),
    # The licences of third-party code compiled into the binary (see NOTICE).
    (os.path.join(root, "LICENSES", "MIT-CORE-MATH.txt"), "LICENSES/MIT-CORE-MATH.txt", 0o644),
    (os.path.join(root, "LICENSES", "MIT-EXPAT.txt"), "LICENSES/MIT-EXPAT.txt", 0o644),
] + [
    (os.path.join(fonts, f), f"LICENSES/{f}", 0o644)
    for f in sorted(os.listdir(fonts))
    if f.startswith("OFL-") and f.endswith(".txt")
]
if target.startswith("x86_64") and not compat:
    launcher = "graphviz-pure-launcher" + (".exe" if windows else "")
    files.append((os.path.join(root, "target-launcher", launcher), launcher, 0o755))

assert sum(arc.startswith("LICENSES/OFL-") for _, arc, _ in files) == 3, "expected 3 OFL files"
os.makedirs(out, exist_ok=True)
if windows:
    path = os.path.join(out, name + ".zip")
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
        for src, arc, mode in sorted(files, key=lambda f: f[1]):
            info = zipfile.ZipInfo(f"{name}/{arc}", date_time=(1980, 1, 1, 0, 0, 0))
            info.external_attr = (mode | 0o100000) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(info, open(src, "rb").read())
else:
    path = os.path.join(out, name + ".tar.gz")
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w", format=tarfile.PAX_FORMAT) as t:
        for src, arc, mode in sorted(files, key=lambda f: f[1]):
            data = open(src, "rb").read()
            info = tarfile.TarInfo(f"{name}/{arc}")
            info.size, info.mode, info.mtime = len(data), mode, epoch
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            t.addfile(info, io.BytesIO(data))
    with open(path, "wb") as f, gzip.GzipFile(fileobj=f, mode="wb", mtime=epoch, filename="") as g:
        g.write(buf.getvalue())
print(path)
