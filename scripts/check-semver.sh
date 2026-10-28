#!/usr/bin/env bash
# D1: only the facade and raster promise a stable Rust API. Their rustdoc builds use separate
# dependency graphs for baseline and candidate, so the facade's exact implementation pins may
# change without conflicting with the published baseline's pins.
set -euo pipefail
cd "$(dirname "$0")/.."

mode=${1:-pr}
case "$mode" in
  pr) : "${BASE_SHA:?PR checks need the base commit from the event payload}" ;;
  release) : "${RELEASE_VERSION:?Release checks need the version selected from merged PR labels}" ;;
  *) echo "usage: check-semver.sh [pr|release]" >&2; exit 2 ;;
esac
cargo semver-checks --version | grep -Fx 'cargo-semver-checks 0.51.0'

failed=0
for package in graphviz-pure graphviz-pure-raster; do
  if [ "$mode" = pr ]; then
    baseline=$BASE_SHA
    baseline_args=(--baseline-rev "$baseline")
  else
    # Release readiness compares the full release delta with crates.io, independently of
    # the candidate manifest version. BASELINE_VERSION permits reproducing an old release.
    baseline=${BASELINE_VERSION:-}
    if [ -z "$baseline" ]; then
      baseline=$(curl --fail --silent --show-error --retry 3 \
        "https://index.crates.io/gr/ap/$package" | jq -esr '
          [.[] | select(.yanked | not) | .vers
           | select(test("^[0-9]+\\.[0-9]+\\.[0-9]+$"))]
          | max_by(split(".") | map(tonumber))')
    fi
    baseline_args=(--baseline-version "$baseline")
  fi
  echo "Checking $package against $mode baseline $baseline (breaking=${BREAKING:-false})."
  status=0
  # Check breaking-change lints even if the candidate already bumped its manifest version.
  cargo semver-checks --manifest-path "${SEMVER_MANIFEST:-Cargo.toml}" -p "$package" \
    "${baseline_args[@]}" --release-type minor || status=$?
  case "$status" in
    0) ;;
    100) # Pinned cargo-semver-checks distinguishes lint violations from operational errors.
      if [ "$mode" = release ]; then
        required=major
        case "$baseline" in 0.*) required=minor ;; esac
        echo "$package API requires a $required version bump from $baseline; selected release: $RELEASE_VERSION."
      fi
      if [ "${BREAKING:-false}" != true ]; then
        if [ "$mode" = pr ]; then
          echo "::error::$package breaks the PR base API ($baseline). Add the breaking label to this PR."
        else
          echo "::error::$package breaks its published API ($baseline), but no merged PR carries breaking; release readiness requires a minor bump while 0.x, major from 1.0."
        fi
        failed=1
      elif [ "$mode" = release ] && ! python3 - "$baseline" "$RELEASE_VERSION" <<'PYVERSION'
import sys
baseline, release = (tuple(map(int, version.split("-")[0].split("."))) for version in sys.argv[1:])
breaking_bump = release[0] > baseline[0] or (baseline[0] == release[0] == 0 and release[1] > baseline[1])
sys.exit(0 if breaking_bump else 1)
PYVERSION
      then
        echo "::error::$package needs a breaking bump from $baseline, but the selected release version is $RELEASE_VERSION."
        failed=1
      else
        echo "Breaking API changes in $package accepted by the breaking label${RELEASE_VERSION:+ and release version $RELEASE_VERSION}."
      fi
      ;;
    *)
      echo "::error::cargo-semver-checks could not check $package (exit $status); the breaking label cannot waive a build, download or tool error."
      failed=1
      ;;
  esac
done
exit "$failed"
