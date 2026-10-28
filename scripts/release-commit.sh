#!/usr/bin/env bash
# Builds the release commit on a worktree of the base release-check.sh recorded, and writes
# a thin git bundle of that one commit. Does not push. The commit is authored as
# github-actions[bot], message "Release X.Y.Z". See docs/releasing.md.
#
# The bundle's prerequisite is the base commit, so a checkout that has the base can
# `git fetch` the bundle and check the commit out.
#
# Usage: scripts/release-commit.sh --meta FILE --section FILE --bundle FILE
#   CARGO   the cargo command for bump-version.sh (default cargo)
# When GITHUB_OUTPUT is set, appends version, release_sha and base_sha there.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source-path=SCRIPTDIR
# shellcheck source=release-lib.sh
. scripts/release-lib.sh

meta="" section="" bundle=""
while [ $# -gt 0 ]; do
  case "$1" in
    --meta) meta=${2:?--meta needs a file}; shift ;;
    --section) section=${2:?--section needs a file}; shift ;;
    --bundle) bundle=${2:?--bundle needs a path}; shift ;;
    *) die "unknown argument $1" ;;
  esac
  shift
done
[ -n "$meta" ] && [ -n "$section" ] && [ -n "$bundle" ] || die "usage: scripts/release-commit.sh --meta FILE --section FILE --bundle FILE"
[ -f "$meta" ] && [ -f "$section" ] || die "meta or section file is missing; run scripts/release-check.sh first"

version=$(jq -er '.version' "$meta")
base=$(jq -er '.base_sha' "$meta")
check_version "$version"
[[ "$base" =~ ^[0-9a-f]{40}$ ]] || die "base_sha in $meta is not a commit sha"
git cat-file -e "$base^{commit}" || die "base $base is not in this repository; fetch master first"

author_name='github-actions[bot]'
author_email='41898282+github-actions[bot]@users.noreply.github.com'
root=$(mktemp -d)
git worktree add --detach "$root" "$base"
cleanup() {
  git worktree remove --force "$root" 2> /dev/null || true
  rm -rf "$root"
}
trap cleanup EXIT

scripts/bump-version.sh "$version" --section "$section" --root "$root"
git -C "$root" add -A
git -C "$root" -c user.name="$author_name" -c user.email="$author_email" commit -m "Release $version"
sha=$(git -C "$root" rev-parse HEAD)
[ "$(git -C "$root" rev-parse HEAD^)" = "$base" ] || die "the release commit is not based on $base"
tmp=$(mktemp)
jq --arg sha "$sha" '. + {release_sha: $sha}' "$meta" > "$tmp"
mv "$tmp" "$meta"

mkdir -p "$(dirname "$bundle")"
# A relative bundle path is relative to this repository, not the worktree.
case "$bundle" in
  /*) bundle_path=$bundle ;;
  *) bundle_path=$PWD/$bundle ;;
esac
git -C "$root" bundle create "$bundle_path" "$base..HEAD"
git -C "$root" bundle verify "$bundle_path" > /dev/null
echo "release commit $sha (parent $base), bundle $bundle_path"

if [ -n "${GITHUB_OUTPUT:-}" ]; then
  {
    echo "version=$version"
    echo "release_sha=$sha"
    echo "base_sha=$base"
  } >> "$GITHUB_OUTPUT"
fi
