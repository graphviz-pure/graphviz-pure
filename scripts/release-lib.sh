# shellcheck shell=bash disable=SC2034
# Shared by the release-*.sh scripts (sourced, not run). See docs/releasing.md.
#
# GH is the gh command for anything that writes (default `gh`). An agent sets it to a wrapper that
# runs gh as the bot account, so the pull requests, issues and workflow runs it starts are not the
# owner's: GitHub does not let anyone approve their own pull request.

REPO=graphviz-pure/graphviz-pure
REMOTE_URL=https://github.com/$REPO.git
# The maintainer who approves releases (the `release` environment's reviewer).
OWNER=cloudbring
read -ra GH_CMD <<< "${GH:-gh}"

die() { echo "error: $*" >&2; exit 1; }

# Refuses to write as the owner: the owner approves what these scripts open. The GraphQL viewer
# query also answers for a GitHub App installation token, where GET /user is a 403 (#486).
require_bot() {
  local login
  login=$("${GH_CMD[@]}" api graphql -f query='{viewer{login}}' -q .data.viewer.login || :)
  [ -n "$login" ] || die "cannot tell which account GH runs gh as: set GH to the bot account's gh"
  [ "$login" != "$OWNER" ] ||
    die "GH runs gh as $OWNER, who approves the release: set GH to the bot account's gh"
}

check_version() {
  [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || die "not a SemVer version: $1"
}

# master and the release tags, over HTTPS (works without an SSH agent).
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

# The commits whose runs count for commit $1: $1 itself and, for a squash merge of pull request
# $2, its head if the two have the same tree (nothing landed in between). Callers use the head
# only for a workflow with no run at all on $1.
candidates() {
  echo "$1"
  if [ -n "${2:-}" ]; then
    local head
    head=$(gh pr view "$2" --repo "$REPO" --json headRefOid -q .headRefOid)
    if [ "$head" != "$1" ] && git cat-file -e "$head" 2>/dev/null &&
      [ "$(git rev-parse "$1^{tree}")" = "$(git rev-parse "$head^{tree}")" ]; then
      echo "$head"
    fi
  fi
}
