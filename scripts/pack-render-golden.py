#!/usr/bin/env python3
"""Packs the render-fidelity references into tests/golden/render.tar.xz.

The references (C's layouts and renderings, about 50 MB) are stored compressed (about 4 MB).
`gv-diff render` unpacks the archive into tests/golden/render/ on first use and whenever the
archive changes; the unpacked directories are gitignored. Only the baselines in
tests/golden/render/*.json are tracked as plain files.

The archive is deterministic (sorted names, zero timestamps and owners), so regenerating
unchanged references leaves it byte for byte the same.

Usage: scripts/pack-render-golden.py   (after scripts/gen-render-golden.sh)
"""
import io
import lzma
import os
import sys
import tarfile

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "tests", "golden"))
SRC = os.path.join(ROOT, "render")
OUT = os.path.join(ROOT, "render.tar.xz")
FORMATS = ["laid", "cmapx", "dot", "json", "json0", "plain", "plain-ext", "svg", "svg_inline", "xdot"]


def main():
    buf = io.BytesIO()
    count = 0
    with tarfile.open(fileobj=buf, mode="w", format=tarfile.USTAR_FORMAT) as tar:
        for fmt in FORMATS:
            d = os.path.join(SRC, fmt)
            if not os.path.isdir(d):
                sys.exit(f"missing {d}; run scripts/gen-render-golden.sh first")
            for name in sorted(os.listdir(d)):
                path = os.path.join(d, name)
                data = open(path, "rb").read()
                info = tarfile.TarInfo(f"render/{fmt}/{name}")
                info.size = len(data)
                info.mode = 0o644
                info.mtime = 0
                tar.addfile(info, io.BytesIO(data))
                count += 1
    data = lzma.compress(buf.getvalue(), preset=9 | lzma.PRESET_EXTREME)
    with open(OUT, "wb") as f:
        f.write(data)
    print(f"packed {count} files into {OUT} ({len(data) / 1e6:.1f} MB)")


if __name__ == "__main__":
    main()
