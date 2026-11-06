#!/usr/bin/env python3
"""Which publishable crates still need their manifest versions published to crates.io.

Reads `cargo metadata --no-deps --format-version 1` for the main workspace (METADATA) and
optionally for the nested rendering-fork workspace (VENDOR_METADATA). Every publishable crate
in both workspaces must have VERSION; fork manifests also name their upstream versions in
their descriptions. Every crate name must already exist and have EXPECTED_OWNER as an owner,
so a dry run fails before a missing fork name can cause a partial publish. Prints GitHub
Actions outputs:

    exclude=--exclude a --exclude b    (main crates already published)
    remaining=N                        (main crates still to publish)
    vendor_exclude=--exclude a ...     (raster forks already published)
    vendor_remaining=N                 (raster forks still to publish)

The release workflow publishes the vendor workspace first. Re-running after a partial publish
leaves out every name/version already present. Python standard library only.

Usage: scripts/crates-to-publish.py METADATA VERSION [VENDOR_METADATA]
"""
import json
import sys
import urllib.error
import urllib.request

EXPECTED_OWNER = "cloudbring"
API = "https://crates.io/api/v1/crates"


def publishable(path):
    meta = json.load(open(path))
    return sorted((p for p in meta["packages"] if p["publish"] is None), key=lambda p: p["name"])


def api_json(url):
    # crates.io's crawler policy asks for a User-Agent that identifies the client.
    req = urllib.request.Request(url, headers={"User-Agent": "graphviz-pure release workflow (https://github.com/graphviz-pure/graphviz-pure)"})
    with urllib.request.urlopen(req, timeout=30) as response:
        return json.load(response)


def check_crate_setup(crates):
    problems = []
    for package in crates:
        name = package["name"]
        try:
            api_json(f"{API}/{name}")
        except urllib.error.HTTPError as e:
            if e.code != 404:
                raise
            problems.append(f"{name}: crate name is not registered")
            continue
        owners = api_json(f"{API}/{name}/owners")
        if not any(user.get("login") == EXPECTED_OWNER for user in owners.get("users", [])):
            problems.append(f"{name}: {EXPECTED_OWNER} is not an owner")
    if problems:
        print("\n".join(problems), file=sys.stderr)
        sys.exit("crates.io setup is incomplete; reserve the names and configure their owners (docs/releasing.md)")


def already_published(crates):
    published = []
    for package in crates:
        name, version = package["name"], package["version"]
        try:
            api_json(f"{API}/{name}/{version}")
            published.append(name)
        except urllib.error.HTTPError as e:
            if e.code != 404:
                raise
        print(f"{name} {version}: {'already published' if name in published else 'to publish'}", file=sys.stderr)
    return published


def main():
    if len(sys.argv) not in (3, 4):
        sys.exit("usage: scripts/crates-to-publish.py METADATA VERSION [VENDOR_METADATA]")
    crates = publishable(sys.argv[1])
    version = sys.argv[2]
    if not crates:
        sys.exit("no publishable crates in the main metadata")
    vendor_crates = publishable(sys.argv[3]) if len(sys.argv) == 4 else []
    if len(sys.argv) == 4 and not vendor_crates:
        sys.exit("no publishable crates in the vendor metadata")
    wrong_version = [p["name"] for p in [*crates, *vendor_crates] if p["version"] != version]
    if wrong_version:
        sys.exit(f"main and vendor crates do not have release version {version}: {', '.join(wrong_version)}")

    check_crate_setup([*crates, *vendor_crates])
    published = already_published(crates)
    vendor_published = already_published(vendor_crates)
    print("exclude=" + " ".join(f"--exclude {n}" for n in published))
    print(f"remaining={len(crates) - len(published)}")
    print("vendor_exclude=" + " ".join(f"--exclude {n}" for n in vendor_published))
    print(f"vendor_remaining={len(vendor_crates) - len(vendor_published)}")


if __name__ == "__main__":
    main()
