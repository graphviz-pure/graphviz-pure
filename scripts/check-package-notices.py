#!/usr/bin/env python3
"""Checks that packaged artifacts carry the licence files (#139): LICENSE, NOTICE and, where
third-party code is compiled in, THIRD-PARTY-LICENSES.md and LICENSES/. Each file must be in the
artifact and match the repository's copy (line endings aside: the Windows archives are built from
a CRLF checkout), so a missing file or a stale copy fails: each crate directory holds copies of
LICENSE and NOTICE, which must be updated with the root files.

Artifacts, recognised by name:
  graphviz-pure-VERSION-TARGET.{tar.gz,zip}   a release archive (scripts/release-package.py)
  graphviz-pure-wasm-VERSION.tgz              the npm package; JSR publishes the files of it
                                              that its jsr.json includes, so those are checked too
  NAME-VERSION.crate                          a crates.io package (cargo package)

Usage: scripts/check-package-notices.py ARTIFACT...
"""
import json
import os
import re
import sys
import tarfile
import zipfile

root = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
fonts = ["crates/graphviz-pure-raster/fonts/OFL-" + f + ".txt" for f in ("Arimo", "Cousine", "Tinos")]

# Path in the artifact -> path in the repository.
BASE = {"LICENSE": "LICENSE", "NOTICE": "NOTICE"}
BINARY = {
    **BASE,
    "THIRD-PARTY-LICENSES.md": "THIRD-PARTY-LICENSES.md",
    "LICENSES/MIT-CORE-MATH.txt": "LICENSES/MIT-CORE-MATH.txt",
    "LICENSES/MIT-EXPAT.txt": "LICENSES/MIT-EXPAT.txt",
}
ARCHIVE = {**BINARY, **{"LICENSES/" + os.path.basename(f): f for f in fonts}}
# Main crates beyond LICENSE and NOTICE (copies of the root files, kept in each crate's directory):
# the ones holding the CORE-MATH and expat ports and the one bundling the fonts. A crate ships sources, not
# its dependencies, so THIRD-PARTY-LICENSES.md is not in any.
CRATES = {
    "graphviz-pure-core": {"LICENSES/MIT-CORE-MATH.txt": "LICENSES/MIT-CORE-MATH.txt"},
    "graphviz-pure-geom": {"LICENSES/MIT-EXPAT.txt": "LICENSES/MIT-EXPAT.txt"},
    "graphviz-pure-raster": {"fonts/" + os.path.basename(f): f for f in fonts},
}
# The raster forks keep their upstream licences rather than the main project's EPL licence.
# Their READMEs name the upstream release and are checked too, so attribution cannot be dropped.
VENDORED_CRATES = {
    "graphviz-pure-tiny-skia": {"LICENSE": "vendor/tiny-skia/LICENSE", "README.md": "vendor/tiny-skia/README.md"},
    "graphviz-pure-tiny-skia-path": {"LICENSE": "vendor/tiny-skia-path/LICENSE", "README.md": "vendor/tiny-skia-path/README.md"},
    "graphviz-pure-kurbo": {"LICENSE-APACHE": "vendor/kurbo/LICENSE-APACHE", "LICENSE-MIT": "vendor/kurbo/LICENSE-MIT", "README.md": "vendor/kurbo/README.md"},
    "graphviz-pure-svgtypes": {"LICENSE-APACHE": "vendor/svgtypes/LICENSE-APACHE", "LICENSE-MIT": "vendor/svgtypes/LICENSE-MIT", "README.md": "vendor/svgtypes/README.md"},
    "graphviz-pure-skrifa": {"LICENSE-APACHE": "vendor/skrifa/LICENSE-APACHE", "LICENSE-MIT": "vendor/skrifa/LICENSE-MIT", "README.md": "vendor/skrifa/README.md"},
    "graphviz-pure-resvg": {"LICENSE-APACHE": "vendor/resvg/LICENSE-APACHE", "LICENSE-MIT": "vendor/resvg/LICENSE-MIT", "README.md": "vendor/resvg/README.md"},
    "graphviz-pure-usvg": {"LICENSE-APACHE": "vendor/usvg/LICENSE-APACHE", "LICENSE-MIT": "vendor/usvg/LICENSE-MIT", "README.md": "vendor/usvg/README.md"},
    "graphviz-pure-svg2pdf": {"LICENSE-APACHE": "vendor/svg2pdf/LICENSE-APACHE", "LICENSE-MIT": "vendor/svg2pdf/LICENSE-MIT", "NOTICE": "vendor/svg2pdf/NOTICE", "README.md": "vendor/svg2pdf/README.md"},
}


def members(path):
    """The artifact's regular files, path -> bytes."""
    if path.endswith(".zip"):
        with zipfile.ZipFile(path) as z:
            return {i.filename: z.read(i) for i in z.infolist() if not i.is_dir()}
    with tarfile.open(path, "r:gz") as t:
        return {m.name: t.extractfile(m).read() for m in t.getmembers() if m.isfile()}


def required(name):
    """(top directory, {path in artifact: path in repository}) for an artifact file name."""
    m = re.fullmatch(r"graphviz-pure-wasm-[^/]+\.tgz", name)
    if m:
        return "package", BINARY
    m = re.fullmatch(r"(graphviz-pure-[0-9][^/]*)\.(tar\.gz|zip)", name)
    if m:
        return m.group(1), ARCHIVE
    m = re.fullmatch(r"(([a-z0-9_-]+)-[0-9][^/]*)\.crate", name)
    if m:
        crate = m.group(2)
        if crate in VENDORED_CRATES:
            return m.group(1), VENDORED_CRATES[crate]
        return m.group(1), {**BASE, **CRATES.get(crate, {})}
    sys.exit(f"error: {name}: not a release archive, npm package or crate")


if len(sys.argv) < 2:
    sys.exit("usage: scripts/check-package-notices.py ARTIFACT...")
errors = 0
for path in sys.argv[1:]:
    top, files = required(os.path.basename(path))
    have = members(path)
    before = errors
    for arc, src in sorted(files.items()):
        with open(os.path.join(root, src), "rb") as f:
            want = f.read().replace(b"\r\n", b"\n")
        got = have.get(f"{top}/{arc}")
        if got is None:
            print(f"error: {path}: {arc} is missing", file=sys.stderr)
            errors += 1
        elif got.replace(b"\r\n", b"\n") != want:
            hint = f"; copy {src} over the crate's copy" if path.endswith(".crate") else ""
            print(f"error: {path}: {arc} differs from {src}{hint}", file=sys.stderr)
            errors += 1
    if top == "package":
        jsr = have.get("package/jsr.json")
        if jsr is None:
            print(f"error: {path}: jsr.json is missing", file=sys.stderr)
            errors += 1
        else:
            include = json.loads(jsr)["publish"]["include"]
            for arc in sorted(set(files) - set(include)):
                print(f"error: {path}: jsr.json does not publish {arc}", file=sys.stderr)
                errors += 1
    if errors == before:
        print(f"{path}: {', '.join(sorted(files))}")
if errors:
    sys.exit(1)
