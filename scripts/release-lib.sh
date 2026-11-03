# shellcheck shell=bash
# Shared by the release-*.sh scripts (sourced, not run). See docs/releasing.md.

REPO=graphviz-pure/graphviz-pure
REMOTE_URL=https://github.com/$REPO.git

# gh colors its JSON when a caller forces a TTY. The scripts pipe that JSON to jq.
export NO_COLOR=1
export GH_FORCE_TTY=0

die() { echo "error: $*" >&2; exit 1; }

check_version() {
  [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || die "not a SemVer version: $1"
}

# master and the release tags, over HTTPS (works without a credential).
fetch_master() {
  git fetch --quiet "$REMOTE_URL" '+refs/heads/master:refs/remotes/origin/master' '+refs/tags/v*:refs/tags/v*'
}

# The newest release tag reachable from $1, leaving out v$2 (the release being made).
prev_tag() {
  git describe --tags --abbrev=0 --match 'v[0-9]*' --exclude "v$2" "$1"
}

# The version in Cargo.toml's [workspace.package] at commit $1.
cargo_version_at() {
  git show "$1:Cargo.toml" | awk -F'"' '/^\[workspace.package\]/{p=1} p && /^version =/{print $2; exit}'
}

# Publish GitHub release $2 (the tag for version $1) from the archives in $3, with notes in $4.
# A hyphenated version (the accepted suffixes are -alpha.N, -beta.N and -rc.N, and any other
# suffix check_version accepts) is a GitHub pre-release. The flag is set on the draft and again
# on the edit that publishes it: `gh release create` sends prerelease from its flag (default
# false), and `gh release edit` sends it only when the flag is passed. A stable X.Y.Z is sent
# as prerelease false. A release that is already published is left unchanged.
publish_github_release() {
  local version=$1 tag=$2 dist=$3 notes=$4 pre
  case "$version" in
    *-*) pre=--prerelease=true ;;
    *) pre=--prerelease=false ;;
  esac
  if gh release view "$tag" --repo "$REPO" --json isDraft -q .isDraft > /dev/null 2>&1; then
    local draft
    draft=$(gh release view "$tag" --repo "$REPO" --json isDraft -q .isDraft)
    if [ "$draft" = true ]; then
      gh release upload "$tag" --repo "$REPO" --clobber "$dist"/*
      gh release edit "$tag" --repo "$REPO" --draft=false --notes-file "$notes" "$pre"
      echo "published the draft $tag"
    else
      echo "$tag is already published"
    fi
  else
    # Immutable once published: attach every file to a draft, then publish.
    gh release create "$tag" --repo "$REPO" --verify-tag --draft "$pre" \
      --title "graphviz-pure $version" --notes-file "$notes" "$dist"/*
    gh release edit "$tag" --repo "$REPO" --draft=false "$pre"
    echo "published $tag"
  fi
}

# "STATUS CONCLUSION URL" of the newest run of workflow $1 on commit $2 (event $3 if given), or
# nothing if there is none.
latest_run() {
  gh run list --repo "$REPO" --workflow "$1" --commit "$2" --limit 20 \
    --json status,conclusion,url,event,createdAt |
    jq -r --arg e "${3:-}" '[.[] | select($e == "" or .event == $e)] | sort_by(.createdAt) | last
      | select(. != null) | "\(.status) \(if (.conclusion // "") == "" then "-" else .conclusion end) \(.url)"'
}
