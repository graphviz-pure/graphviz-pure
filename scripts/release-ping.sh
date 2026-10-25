#!/usr/bin/env bash
# The release's one issue, "Release VERSION ready": it tells the owner what to run, step by step,
# and the agent driving the release adds each next command as a comment. Run with GH set to the bot
# account's gh: the owner is assigned and mentioned, and the issue must not come from the owner.
#
#   release-ping.sh VERSION PR [--force]  opens the issue once scripts/release-ready.sh VERSION PR
#                                         passes (--force: open it anyway, checklist included)
#   release-ping.sh VERSION tag           comments the tag command for the release PR's merge commit
#   release-ping.sh VERSION deploy        comments how to approve the waiting `release` deployment
#   release-ping.sh VERSION npm           comments `npm stage approve <id>`, read from the npm job's log
#   release-ping.sh VERSION done          runs scripts/release-verify.sh, comments its result and
#                                         closes the issue if every channel passes
#
# See docs/releasing.md.
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/release-lib.sh

version=${1:?usage: scripts/release-ping.sh VERSION PR|tag|deploy|npm|done}
step=${2:?usage: scripts/release-ping.sh VERSION PR|tag|deploy|npm|done}
check_version "$version"
title="Release $version ready"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

require_bot
issue=$(gh issue list --repo "$REPO" --state open --search "in:title \"$title\"" --json number,title \
  -q "map(select(.title == \"$title\")) | first | .number // empty")
comment() { "${GH_CMD[@]}" issue comment "$issue" --repo "$REPO" --body-file "$work/comment.md"; }
release_run() {
  gh run list --repo "$REPO" --workflow release.yml --branch "v$version" --event push --limit 1 \
    --json databaseId -q '.[0].databaseId // empty'
}

case "$step" in
  [0-9]*)
    pr=$step
    [ -z "$issue" ] || die "#$issue is already open"
    ready=0
    scripts/release-ready.sh "$version" "$pr" > "$work/ready.md" || ready=$?
    if [ "$ready" != 0 ] && [ "${3:-}" != --force ]; then
      cat "$work/ready.md"
      die "not ready (--force opens the issue anyway)"
    fi
    url=$(gh pr view "$pr" --repo "$REPO" --json url -q .url)
    dry=$(grep -F 'release dry run' "$work/ready.md" | grep -o 'https://github.com/[^ ]*/actions/runs/[0-9]*' || true)
    cat > "$work/body.md" <<EOF
@$OWNER, graphviz-pure $version is ready to release.

- Release pull request: $url
- Dry run of the release workflow on it (built, smoke-tested and attested, nothing published): ${dry:-none yet}

## Your four steps

1. **Approve the release pull request.** The agent then adds it to the merge queue.
   \`\`\`sh
   gh pr review $pr --repo $REPO --approve
   \`\`\`
2. **Sign and push the tag** on the merge commit, which the agent comments here once #$pr merges:
   \`\`\`sh
   git fetch origin master && git tag -s v$version -m v$version <merge sha> && git push origin v$version
   \`\`\`
3. **Approve the \`release\` deployment** of the release run: one approval starts every publishing job (GitHub release, crates.io, npm, JSR, Wasmer, PyPI, ghcr). The agent comments the run and the command here when it waits.
4. **Approve the staged npm version** with 2FA: the agent comments \`npm stage approve <id>\` here once the npm job has staged it.

The agent then checks every channel (\`scripts/release-verify.sh $version\`), starts the Homebrew tap and Scoop bucket updates, and closes this issue.

$(cat "$work/ready.md")
EOF
    "${GH_CMD[@]}" issue create --repo "$REPO" --title "$title" --assignee "$OWNER" --body-file "$work/body.md"
    ;;
  tag)
    [ -n "$issue" ] || die "no open \"$title\" issue"
    read -r state merge < <(gh pr view "release/v$version" --repo "$REPO" --json state,mergeCommit \
      -q '"\(.state) \(.mergeCommit.oid // "-")"')
    [ "$state" = MERGED ] || die "the release pull request is $state, not merged"
    cat > "$work/comment.md" <<EOF
@$OWNER: the release pull request merged as \`$merge\`. Step 2, sign and push the tag:

\`\`\`sh
git fetch origin master && git tag -s v$version -m v$version $merge && git push origin v$version
\`\`\`
EOF
    comment
    ;;
  deploy)
    [ -n "$issue" ] || die "no open \"$title\" issue"
    run=$(release_run)
    [ -n "$run" ] || die "no release run for v$version yet"
    env_id=$(gh api "repos/$REPO/environments/release" -q .id)
    cat > "$work/comment.md" <<EOF
@$OWNER: the release run waits for the \`release\` environment: https://github.com/$REPO/actions/runs/$run. Every smoke test passed. Step 3, approve it once (Review deployments → release → Approve and deploy), or:

\`\`\`sh
gh api --method POST repos/$REPO/actions/runs/$run/pending_deployments -F 'environment_ids[]=$env_id' -f state=approved -f comment='Release $version'
\`\`\`
EOF
    comment
    ;;
  npm)
    [ -n "$issue" ] || die "no open \"$title\" issue"
    run=$(release_run)
    [ -n "$run" ] || die "no release run for v$version yet"
    job=$(gh run view "$run" --repo "$REPO" --json jobs -q '.jobs[] | select(.name == "Publish to npm") | .databaseId')
    # Through the API: `gh run view --log` refuses while the run (its verify job) is still going.
    id=$(gh api --allow-escape-sequences "repos/$REPO/actions/jobs/$job/logs" |
      grep -oE 'staged with id [0-9a-f-]+' | awk '{print $4}' | tail -n 1 || true)
    [ -n "$id" ] || die "the npm job's log has no stage id"
    cat > "$work/comment.md" <<EOF
@$OWNER: @graphviz-pure/wasm $version is staged on npm. Step 4, approve it with 2FA, then log out:

\`\`\`sh
npm stage approve $id && npm logout
\`\`\`
EOF
    comment
    ;;
  done)
    [ -n "$issue" ] || die "no open \"$title\" issue"
    passed=0
    scripts/release-verify.sh "$version" > "$work/comment.md" || passed=$?
    comment
    if [ "$passed" = 0 ]; then
      "${GH_CMD[@]}" issue close "$issue" --repo "$REPO" --reason completed
    else
      cat "$work/comment.md"
      die "a channel fails: the issue stays open"
    fi
    ;;
  *) die "unknown step $step" ;;
esac
