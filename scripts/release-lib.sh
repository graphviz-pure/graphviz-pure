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

# "STATUS CONCLUSION URL" of the newest run of workflow $1 on commit $2 (event $3 if given), or
# nothing if there is none.
latest_run() {
  gh run list --repo "$REPO" --workflow "$1" --commit "$2" --limit 20 \
    --json status,conclusion,url,event,createdAt |
    jq -r --arg e "${3:-}" '[.[] | select($e == "" or .event == $e)] | sort_by(.createdAt) | last
      | select(. != null) | "\(.status) \(if (.conclusion // "") == "" then "-" else .conclusion end) \(.url)"'
}
