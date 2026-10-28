#!/usr/bin/env bash
# Sets VERSION in Cargo.toml (the workspace version and the version requirements of the
# workspace's own crates), web/package.json, web/jsr.json and Cargo.lock, and inserts the
# changelog section for VERSION. Does not commit. The release workflow runs this on a worktree
# of master; see docs/releasing.md.
#
# Usage: scripts/bump-version.sh VERSION --section FILE [--root DIR]
#   --section FILE   the rendered section, including its "## [VERSION] - date" heading
#                    (scripts/release-changelog.py render). CHANGELOG.md's Unreleased section
#                    must be empty: entries belong in pull request bodies.
#   --root DIR       the tree to edit (default: this repository)
#   CARGO            the cargo command for `cargo update` (default cargo)
set -euo pipefail
read -ra cargo <<< "${CARGO:-cargo}"

version="" section="" root=""
while [ $# -gt 0 ]; do
  case "$1" in
    --section) section=${2:?--section needs a file}; shift ;;
    --root) root=${2:?--root needs a directory}; shift ;;
    -*) echo "unknown option $1" >&2; exit 1 ;;
    *) version=$1 ;;
  esac
  shift
done
[ -n "$version" ] && [ -n "$section" ] || { echo "usage: scripts/bump-version.sh VERSION --section FILE [--root DIR]" >&2; exit 1; }
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || { echo "not a SemVer version: $version" >&2; exit 1; }
[ -f "$section" ] || { echo "no such section file: $section" >&2; exit 1; }
root=${root:-$(cd "$(dirname "$0")/.." && pwd)}
cd "$root"
old=$(awk -F'"' '/^\[workspace.package\]/{p=1} p && /^version =/{print $2; exit}' Cargo.toml)
[ -n "$old" ] || { echo "Cargo.toml has no workspace version" >&2; exit 1; }
grep -q '^## \[Unreleased\]' CHANGELOG.md || { echo "CHANGELOG.md has no Unreleased section" >&2; exit 1; }

export VERSION_FILE=$section
python3 - "$old" "$version" <<'PY'
import json, os, re, sys
old, new = sys.argv[1], sys.argv[2]
section = open(os.environ["VERSION_FILE"]).read().strip("\n") + "\n"
heading = f"## [{new}] - "
if not section.startswith(heading):
    sys.exit(f"the section does not start with {heading!r}")
def bump_reqs(text, new):
    # Keep a leading '=' so the facade's exact pins stay exact.
    return re.sub(
        r'(graphviz-pure[\w-]* = \{ path = "[^"]+", version = ")(=?)[^"]+(")',
        rf"\g<1>\g<2>{new}\g<3>",
        text,
    )

toml = open("Cargo.toml").read()
toml = re.sub(r'(\[workspace\.package\][^\[]*?\nversion = ")[^"]+(")', rf"\g<1>{new}\g<2>", toml, count=1)
toml = bump_reqs(toml, new)
open("Cargo.toml", "w").write(toml)
facade = "crates/graphviz-pure/Cargo.toml"
facade_text = open(facade).read()
open(facade, "w").write(bump_reqs(facade_text, new))
for path in ("web/package.json", "web/jsr.json"):
    pkg = json.load(open(path))
    pkg["version"] = new
    open(path, "w").write(json.dumps(pkg, indent=2) + "\n")
log = open("CHANGELOG.md").read()
m = re.search(r"^## \[Unreleased\]\n(.*?)(?=^## \[)", log, re.M | re.S)
if not m:
    sys.exit("CHANGELOG.md has no Unreleased section")
if m.group(1).strip():
    sys.exit("CHANGELOG.md's Unreleased section is not empty: entries belong in pull request bodies")
log = log[:m.start(1)] + "\n" + section + "\n" + log[m.end(1):]
base = "https://github.com/graphviz-pure/graphviz-pure"
links = re.compile(r"^\[Unreleased\]: .*$", re.M)
prev = re.search(r"^\[(\d[^\]]*)\]: ", log, re.M)
log = links.sub(
    f"[Unreleased]: {base}/compare/v{new}...HEAD\n"
    + (f"[{new}]: {base}/compare/v{prev.group(1)}...v{new}" if prev else f"[{new}]: {base}/releases/tag/v{new}"),
    log, count=1)
open("CHANGELOG.md", "w").write(log)
PY
"${cargo[@]}" update --workspace --quiet
echo "version $old -> $version"
git --no-pager diff --stat
