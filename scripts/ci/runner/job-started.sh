#!/usr/bin/env bash
# The runner's job-started hook (ACTIONS_RUNNER_HOOK_JOB_STARTED, set in the image, not by the
# workflow). It runs before the job's first step and fails the job unless the job may run on
# this machine: a push, a merge-queue run, a manual run or a pull request from a branch of this
# repository. A fork's pull request can edit the workflow it runs, including its runs-on, so the
# workflows' runs-on expression is not enough on its own; this check is the one the pull request
# cannot change. See docs/ci/self-hosted-runners.md.
set -euo pipefail

allowed_repo=graphviz-pure/graphviz-pure
event=${GITHUB_EVENT_NAME:-}
repo=${GITHUB_REPOSITORY:-}

deny() {
  echo "::error::This self-hosted runner does not run $1. Fork pull requests run on GitHub-hosted runners."
  exit 1
}

[ "$repo" = "$allowed_repo" ] || deny "jobs of $repo"
case "$event" in
  push | merge_group | workflow_dispatch) ;;
  pull_request)
    head=$(jq -r '.pull_request.head.repo.full_name // empty' "${GITHUB_EVENT_PATH:?}")
    [ "$head" = "$allowed_repo" ] || deny "pull requests from ${head:-an unknown repository}"
    ;;
  *) deny "the $event event" ;;
esac
echo "gvp-ci: $event of $repo allowed on self-hosted runner ${RUNNER_NAME:-?}"
