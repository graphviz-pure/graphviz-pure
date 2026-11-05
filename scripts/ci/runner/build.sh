#!/usr/bin/env bash
# Builds the self-hosted runner image from this directory: gvp-ci-runner:<runner version>-<hash
# of this directory>, also tagged gvp-ci-runner:current, which the supervisor starts.
set -euo pipefail
dir=$(cd "$(dirname "$0")" && pwd)
version=$(awk -F= '/^ARG RUNNER_VERSION=/ { print $2 }' "$dir/Dockerfile")
hash=$(cd "$dir" && cat Dockerfile sudo-shim.sh job-started.sh entrypoint.sh | sha256sum | cut -c1-12)
tag="gvp-ci-runner:$version-$hash"
docker build --pull -t "$tag" "$dir"
docker tag "$tag" gvp-ci-runner:current
echo "$tag"
