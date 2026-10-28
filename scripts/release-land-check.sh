#!/usr/bin/env bash
# Can this release commit land? Runs in prepare, after the commit exists and before any job
# can request the release environment. Reads only. See docs/releasing.md.
#
#   - git merge-tree --write-tree of origin/master and the release commit is clean
#     (master may have moved; a conflict fails);
#   - refs/tags/vX.Y.Z does not exist;
#   - no GitHub release, including a draft, exists for vX.Y.Z.
#
# A missing App secret or a ruleset bypass that rejects the App cannot be checked here.
#
# Usage: scripts/release-land-check.sh --meta FILE [--summary FILE]
#   --meta FILE   release-commit.sh's meta, including release_sha
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source-path=SCRIPTDIR
# shellcheck source=release-lib.sh
. scripts/release-lib.sh

meta="" summary=""
while [ $# -gt 0 ]; do
  case "$1" in
    --meta) meta=${2:?--meta needs a file}; shift ;;
    --summary) summary=${2:?--summary needs a file}; shift ;;
    *) die "unknown argument $1" ;;
  esac
  shift
done
[ -n "$meta" ] && [ -f "$meta" ] || die "usage: scripts/release-land-check.sh --meta FILE [--summary FILE]"

version=$(jq -er '.version' "$meta")
release=$(jq -er '.release_sha' "$meta")
check_version "$version"
[[ "$release" =~ ^[0-9a-f]{40}$ ]] || die "release_sha in $meta is not a commit sha"
git cat-file -e "$release^{commit}" || die "release commit $release is not in this repository"

fetch_master
tag=v$version
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fails=0
detail=""

# Authoritative: a tag that exists only on the remote, including one a draft release created.
# ls-remote --exit-code returns 2 when the ref is absent and 0 when it is present.
set +e
git ls-remote --exit-code "$REMOTE_URL" "refs/tags/$tag" > /dev/null 2>&1
ls_status=$?
set -e
if [ "$ls_status" -eq 0 ]; then
  fails=$((fails + 1))
  detail="${detail}- [ ] \`$tag\` does not exist — the tag is already on the remote
"
elif [ "$ls_status" -ne 2 ]; then
  die "could not list refs/tags/$tag (git ls-remote exited $ls_status)"
else
  detail="${detail}- [x] \`$tag\` does not exist
"
fi

# gh api writes a JSON error body on 404 and exits non-zero. A draft is returned when this
# token can see it (the job's read-only contents token).
if gh api "repos/$REPO/releases/tags/$tag" > "$work/rel.json" 2> /dev/null; then
  draft=$(jq -r '.draft' "$work/rel.json")
  fails=$((fails + 1))
  detail="${detail}- [ ] No GitHub release for \`$tag\` — one exists (draft=$draft)
"
else
  listed=$(gh release list --repo "$REPO" --limit 100 --json tagName,isDraft |
    jq -r --arg t "$tag" '.[] | select(.tagName == $t) | "draft=\(.isDraft)"')
  if [ -n "$listed" ]; then
    fails=$((fails + 1))
    detail="${detail}- [ ] No GitHub release for \`$tag\` — $listed
"
  else
    detail="${detail}- [x] No GitHub release for \`$tag\`
"
  fi
fi

if git merge-tree --write-tree origin/master "$release" > "$work/tree" 2> "$work/tree.err"; then
  detail="${detail}- [x] \`${release:0:12}\` merges cleanly onto origin/master
"
else
  fails=$((fails + 1))
  conflict=$(head -n 8 "$work/tree" "$work/tree.err" | tr '\n' ' ' | cut -c 1-400)
  detail="${detail}- [ ] \`${release:0:12}\` merges cleanly onto origin/master — $conflict
"
fi

{
  echo
  echo "## Landing"
  echo
  printf '%s' "$detail"
  if [ "$fails" = 0 ]; then
    echo
    echo "**Lands:** $tag from ${release:0:12} onto $(git rev-parse --short origin/master)."
  else
    echo
    echo "**Does not land:** $fails item(s) open. Nothing will be published."
  fi
} > "$work/report.md"
cat "$work/report.md"
[ -z "$summary" ] || cat "$work/report.md" >> "$summary"
[ "$fails" = 0 ]
