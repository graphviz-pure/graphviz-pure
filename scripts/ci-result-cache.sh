#!/usr/bin/env bash
# Skips CI work already done for the same inputs.
#
#   ci-result-cache.sh lookup KIND   writes hit=true|false, run_id and key to $GITHUB_OUTPUT
#   ci-result-cache.sh name KIND     prints the marker artifact's name for this checkout
#   ci-result-cache.sh changed KIND BASE
#                                    exits 0 if a file in the KEY differs between BASE and HEAD
#
# A workflow that passes uploads an artifact named pass-KIND-KEY. KEY hashes the path, mode and
# blob of every tracked file the skipped jobs can read, so it names the files tested, not the
# commit. A later run with the same KEY finds the marker and skips the heavy jobs: a squash merge
# of an up-to-date pull request gives master the files its pull_request run tested, and a change
# confined to the paths in `ignored` below (documentation, say) keeps the KEY of its base.
# Artifacts are repository-wide (the Actions cache is per branch, and master cannot read a pull
# request's cache entries).
# Only markers uploaded by runs of this repository count, never a fork's pull request run.
#
# Needs GH_TOKEN (actions: read), GITHUB_REPOSITORY and GITHUB_REPOSITORY_ID.
set -euo pipefail

cmd=${1:?usage: ci-result-cache.sh lookup|name|changed KIND [BASE]}
kind=${2:?usage: ci-result-cache.sh lookup|name|changed KIND [BASE]}

# Paths that no job this script skips reads (ci.yml: verify, ns-check, msrv, wasm-e2e,
# wasm-runtimes; cross-platform.yml: the platform renders and compare). The site job, the
# supply-chain job and the PR risk report always run, so the files only they read are listed
# too. A path not listed here is part of the KEY, so a new file is tested until someone adds it.
#
# site/ is listed except for the samples, which the renders and wasm-runtimes read
# (scripts/cross-platform-render.sh, scripts/cross-platform-wasm.mjs, scripts/wasm-runtimes).
ignored=(
  docs
  site
  '*.md'
  .github/ISSUE_TEMPLATE
  .github/pull_request_template.md
  .github/CODEOWNERS
  .github/allowed_signers
  .github/dependabot.yml
  .github/workflows/release.yml
  .github/workflows/scorecard.yml
  # The cargo-fuzz crate is its own workspace; fuzz.yml builds and runs it.
  fuzz
  .clusterfuzzlite
  .github/workflows/fuzz.yml
  .gitignore
  about.toml
  deny.toml
  packaging
  web/README.md
  # Run by hand, by the release, or by a job that always runs.
  scripts/bump-version.sh
  scripts/c-oracle-linux
  scripts/ci-stats
  scripts/ci-report-failure.sh
  'scripts/*-probe.c'
  scripts/cargo-locked.sh
  scripts/crates-to-publish.py
  'scripts/dot-*.sh'
  scripts/file-issues.py
  'scripts/fuzz-*.py'
  'scripts/gen-*'
  scripts/gen_color_table.py
  scripts/pack-render-golden.py
  scripts/pr-risk.py
  'scripts/release-*'
  'scripts/reserve-*.sh'
  'scripts/site-*'
  'scripts/third-party-licenses.*'
  scripts/triage-pr.sh
  scripts/update-baselines.sh
  scripts/verify-local.sh
)
read_by_jobs=(site/public/samples site/src/data/samples.json)
# Unlike distribution recipes, the launcher is compiled by CPU fidelity and Windows cross-platform checks.
read_by_jobs+=(packaging/launcher)

# The index, which matches HEAD in a fresh checkout. glob magic: `*` does not cross a `/`.
key=$({
  git ls-files --stage -- . "${ignored[@]/#/:(exclude,glob)}"
  git ls-files --stage -- "${read_by_jobs[@]}"
} | git hash-object --stdin)
name="pass-$kind-$key"

case "$cmd" in
  name)
    echo "$name"
    ;;
  changed)
    base=${3:?usage: ci-result-cache.sh changed KIND BASE}
    ! git diff --quiet "$base" HEAD -- . "${ignored[@]/#/:(exclude,glob)}" ||
      ! git diff --quiet "$base" HEAD -- "${read_by_jobs[@]}"
    ;;
  lookup)
    run_id=$(gh api "repos/$GITHUB_REPOSITORY/actions/artifacts?name=$name&per_page=100" \
      --jq "[.artifacts[] | select(.expired | not)
             | select(.workflow_run.head_repository_id == $GITHUB_REPOSITORY_ID)]
            | first | .workflow_run.id // empty")
    if [ -n "$run_id" ]; then
      echo "Key $key already passed in run $run_id; skipping the jobs it covers."
      { echo "hit=true"; echo "run_id=$run_id"; } >> "$GITHUB_OUTPUT"
    else
      echo "No earlier run passed key $key."
      { echo "hit=false"; echo "run_id="; } >> "$GITHUB_OUTPUT"
    fi
    echo "key=$key" >> "$GITHUB_OUTPUT"
    ;;
  *)
    echo "unknown command: $cmd" >&2
    exit 2
    ;;
esac
