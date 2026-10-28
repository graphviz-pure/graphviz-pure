#!/usr/bin/env bash
# Pushes the release commit to master, tags it, publishes the GitHub release, and dispatches
# the Homebrew tap and Scoop bucket updates. A merge conflict stops the script before it
# pushes, tags, creates a release or dispatches anything. Re-running after a partial success
# continues: a commit already on master is not pushed again, a tag already at the release
# commit is left, and a published release is left published. See docs/releasing.md.
#
# Usage: scripts/release-ship.sh DIST_DIR
#   VERSION, BASE_SHA, RELEASE_SHA   the prepare job's outputs
#   APP_TOKEN                         GitHub App installation token (contents and actions: write)
#
# The checkout must already contain BASE_SHA and RELEASE_SHA (the release-commit bundle).
# master is pushed as a fast-forward when it is still BASE_SHA, otherwise as a merge commit
# of the release commit onto current master. The tag is an unsigned annotated tag of the
# release commit, not of the merge commit. The release notes are that commit's changelog section.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source-path=SCRIPTDIR
# shellcheck source=release-lib.sh
. scripts/release-lib.sh

dist=${1:?usage: scripts/release-ship.sh DIST_DIR}
version=${VERSION:?VERSION is required}
base=${BASE_SHA:?BASE_SHA is required}
release=${RELEASE_SHA:?RELEASE_SHA is required}
token=${APP_TOKEN:?APP_TOKEN is required}
check_version "$version"
[[ "$base" =~ ^[0-9a-f]{40}$ ]] || die "BASE_SHA is not a commit sha"
[[ "$release" =~ ^[0-9a-f]{40}$ ]] || die "RELEASE_SHA is not a commit sha"
[ -d "$dist" ] || die "no such directory: $dist"
git cat-file -e "$release^{commit}" || die "$release is not in this checkout"

author_name='github-actions[bot]'
author_email='41898282+github-actions[bot]@users.noreply.github.com'
url=$REMOTE_URL
tag=v$version
# The same header actions/checkout uses for an installation token. basic, not bearer, and
# the base64 is one line (GNU base64 wraps). The token stays out of the remote URL.
git_auth() {
  local basic
  basic=$(printf 'x-access-token:%s' "$token" | base64 | tr -d '\n')
  git -c "http.https://github.com/.extraheader=AUTHORIZATION: basic $basic" "$@"
}

git_auth fetch --quiet "$url" '+refs/heads/master:refs/remotes/origin/master'
master=$(git rev-parse origin/master)
if [ "$master" = "$base" ]; then
  [ "$(git rev-parse "$release^")" = "$base" ] || die "$release is not a child of $base"
  update=$release
elif git merge-base --is-ancestor "$release" "$master"; then
  update=$master
else
  git checkout --quiet --detach "$master"
  if ! git -c user.name="$author_name" -c user.email="$author_email" \
      merge --no-ff --no-edit -m "Merge release $version" "$release"; then
    git merge --abort || true
    die "merging $release onto master conflicts; nothing was pushed"
  fi
  update=$(git rev-parse HEAD)
fi

if [ "$update" != "$master" ]; then
  git_auth push "$url" "$update:refs/heads/master"
  echo "pushed $update to master"
else
  echo "master already contains $release"
fi

git_auth fetch --quiet "$url" "+refs/tags/$tag:refs/tags/$tag" || true
existing=$(git rev-parse -q --verify "refs/tags/$tag^{}" || true)
if [ -n "$existing" ]; then
  [ "$existing" = "$release" ] || die "$tag points at $existing, not $release"
  echo "$tag already points at $release"
else
  git -c user.name="$author_name" -c user.email="$author_email" -c tag.gpgSign=false \
    tag -a "$tag" "$release" -m "$tag"
  git_auth push "$url" "refs/tags/$tag"
  echo "pushed $tag"
fi

notes=$(mktemp)
trap 'rm -f "$notes"' EXIT
git show "$release:CHANGELOG.md" | awk -v v="## [$version]" \
  'index($0, v) == 1 {p=1} p && /^## \[/ && index($0, v) != 1 {exit} p' > "$notes"
[ -s "$notes" ] || die "CHANGELOG.md at $release has no $version section"

export GH_TOKEN=$token
if gh release view "$tag" --repo "$REPO" --json isDraft -q .isDraft > /dev/null 2>&1; then
  draft=$(gh release view "$tag" --repo "$REPO" --json isDraft -q .isDraft)
  if [ "$draft" = true ]; then
    gh release upload "$tag" --repo "$REPO" --clobber "$dist"/*
    gh release edit "$tag" --repo "$REPO" --draft=false --notes-file "$notes"
    echo "published the draft $tag"
  else
    echo "$tag is already published"
  fi
else
  # Immutable once published: attach every file to a draft, then publish.
  gh release create "$tag" --repo "$REPO" --verify-tag --draft --title "graphviz-pure $version" \
    --notes-file "$notes" "$dist"/*
  gh release edit "$tag" --repo "$REPO" --draft=false
  echo "published $tag"
fi

# The tap and bucket workflows are workflow_dispatch on their default branch. Dispatching
# again is safe: their update exits 0 when the formula or manifest is already current.
gh workflow run update.yml --repo graphviz-pure/homebrew-tap
gh workflow run update.yml --repo graphviz-pure/scoop-bucket
echo "dispatched homebrew-tap and scoop-bucket update.yml"
