#!/usr/bin/env python3
"""Checks that cargo-binstall finds every release archive: for each graphviz-pure-VERSION-TARGET
archive in DIST (scripts/release-package.py; not the WASI one, which cargo cannot install), the
`[package.metadata.binstall]` of MANIFEST (crates/graphviz-pure-cli/Cargo.toml, or the one
published on crates.io) must name its selected archive (compat on x86-64), in the format of its extension, and the binary's
path inside it.

The templates are rendered the way cargo-binstall does (SUPPORT.md): `{ repo }`, `{ version }`,
`{ target }`, `{ archive-suffix }`, `{ bin }` and `{ binary-ext }`, with the target's override
applied over the package's values. It is stricter than cargo-binstall, never looser: other
template keys, other package formats and `cfg(...)` overrides (which it cannot evaluate) fail the
check, so a change to the metadata that this script does not understand is caught here.

Usage: scripts/check-binstall.py MANIFEST DIST VERSION
"""
import os
import re
import sys
import tarfile
import tomllib
import zipfile

manifest, dist, version = sys.argv[1:4]
with open(manifest, "rb") as f:
    cargo = tomllib.load(f)
package = cargo["package"]
meta = package.get("metadata", {}).get("binstall")
if meta is None:
    sys.exit(f"{manifest} has no [package.metadata.binstall]")
repo = package["repository"]
if isinstance(repo, dict):  # repository.workspace = true: the workspace's
    with open(os.path.join(os.path.dirname(os.path.abspath(manifest)), "..", "..", "Cargo.toml"), "rb") as f:
        repo = tomllib.load(f)["workspace"]["package"]["repository"]
bins = [b["name"] for b in cargo.get("bin", [])] or [package["name"]]
suffixes = {"tgz": ".tar.gz", "zip": ".zip"}
overrides = meta.get("overrides", {})
prefix = f"{repo}/releases/download/v{version}/"

archives = sorted(
    f for f in os.listdir(dist)
    if re.fullmatch(rf"graphviz-pure-{re.escape(version)}-(.+)\.(tar\.gz|zip)", f) and "-wasm32-" not in f
)
if not archives:
    sys.exit(f"no graphviz-pure-{version}-TARGET archive in {dist}")
targets = {re.fullmatch(rf"graphviz-pure-{re.escape(version)}-(.+)\.(tar\.gz|zip)", a).group(1) for a in archives}
# CPU variants are not Rust target triples. Keep old releases verifiable too:
# the published manifest decides whether a target selects compat or the ordinary asset.
normal_targets = {t.removesuffix("-compat") for t in targets}

def uses_compat(target):
    return "-compat" in {**meta, **overrides.get(target, {})}["pkg-url"]

bad = []
for target in normal_targets:
    if uses_compat(target) and target + "-compat" not in targets:
        bad.append(f"{target}: missing baseline -compat archive")
bad += [f"override {k!r} is not a release target (cfg() overrides are not checked)" for k in overrides if k not in normal_targets]
selected = 0
for archive in archives:
    variant = re.fullmatch(rf"graphviz-pure-{re.escape(version)}-(.+)\.(tar\.gz|zip)", archive).group(1)
    target = variant.removesuffix("-compat")
    if variant.endswith("-compat") != uses_compat(target):
        continue
    selected += 1
    opts = {**meta, **overrides.get(target, {})}
    fmt = opts.get("pkg-fmt", "tgz")  # binstall's default
    if fmt not in suffixes:
        bad.append(f"{target}: pkg-fmt {fmt!r} is not one this check handles ({', '.join(suffixes)})")
        continue
    values = {
        "repo": repo, "version": version, "target": target, "archive-suffix": suffixes[fmt],
        "binary-ext": ".exe" if "windows" in target else "",
    }

    def render(template, **extra):
        def sub(m):
            key = m.group(1)
            if key not in {**values, **extra}:
                sys.exit(f"{manifest}: unknown template key {{ {key} }}")
            return {**values, **extra}[key]
        return re.sub(r"\{ *([a-z-]+) *\}", sub, template)

    url = render(opts["pkg-url"])
    if url != prefix + archive:
        bad.append(f"{target}: pkg-url gives {url}, not {prefix}{archive}")
        continue
    if not archive.endswith(suffixes[fmt]):
        bad.append(f"{target}: pkg-fmt {fmt} but the archive is {archive}")
        continue
    path = os.path.join(dist, archive)
    names = zipfile.ZipFile(path).namelist() if fmt == "zip" else tarfile.open(path).getnames()
    for b in bins:
        inner = render(opts["bin-dir"], bin=b)
        if inner not in names:
            bad.append(f"{target}: {archive} has no {inner}")
    print(f"{target}: {archive} → {', '.join(render(opts['bin-dir'], bin=b) for b in bins)}")
if bad:
    sys.exit("cargo-binstall would not find:\n  " + "\n  ".join(bad))
print(f"cargo-binstall finds all {selected} selected archives")
