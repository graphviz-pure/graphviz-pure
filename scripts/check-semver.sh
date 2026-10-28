#!/usr/bin/env bash
# D1: only the facade and raster promise a stable Rust API. Their rustdoc builds use separate
# dependency graphs for baseline and candidate, so the facade's exact implementation pins may
# change without conflicting with the published baseline's pins.
set -euo pipefail
cd "$(dirname "$0")/.."

failed=0
for package in graphviz-pure graphviz-pure-raster; do
  # Select the latest non-yanked stable publication, independently of Cargo.toml's version.
  # BASELINE_VERSION is only for reproducing a check against a particular release locally.
  baseline=${BASELINE_VERSION:-}
  if [ -z "$baseline" ]; then
    baseline=$(curl --fail --silent --show-error --retry 3 \
      "https://index.crates.io/gr/ap/$package" | jq -esr '
        [.[] | select(.yanked | not) | .vers
         | select(test("^[0-9]+\\.[0-9]+\\.[0-9]+$"))]
        | max_by(split(".") | map(tonumber))')
  fi
  echo "Checking $package against crates.io $baseline (breaking=${BREAKING:-false})."
  status=0
  # "minor" is the checker's compatibility level: run the lints requiring a breaking bump
  # (minor for 0.x, major from 1.0), even if a PR already changed the manifest's version.
  cargo semver-checks -p "$package" --baseline-version "$baseline" --release-type minor || status=$?
  case "$status" in
    0) ;;
    100) # Pinned cargo-semver-checks distinguishes lint violations from operational errors.
      if [ "${BREAKING:-false}" = true ]; then
        echo "Breaking API changes in $package accepted by the breaking label."
      else
        echo "::error::$package breaks its published Rust API ($baseline). Add the breaking label to this PR: release selection bumps minor while 0.x and major from 1.0."
        failed=1
      fi
      ;;
    *)
      echo "::error::cargo-semver-checks could not check $package (exit $status); the breaking label cannot waive a build, download or tool error."
      failed=1
      ;;
  esac
done
exit "$failed"
