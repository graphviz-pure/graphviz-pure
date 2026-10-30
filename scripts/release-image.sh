#!/usr/bin/env bash
# Builds the linux/amd64 + linux/arm64 container image from a release's static musl
# archives (graphviz-pure-VERSION-{x86_64,aarch64}-unknown-linux-musl.tar.gz in DIST, as
# scripts/release-package.py writes them, plus the x86_64 -compat archive). Nothing is compiled: the image holds the binary and
# the licence files (LICENSE, NOTICE, THIRD-PARTY-LICENSES.md, LICENSES/) from those archives
# (scripts/release-image.Dockerfile).
#
# Writes OUT/graphviz-pure-image.tar, an OCI image layout tagged IMAGE:VERSION, and
# OUT/graphviz-pure-image.digest, the digest of its image index (what gets pushed and attested).
# The build is reproducible: timestamps come from SOURCE_DATE_EPOCH (default: the last
# commit's), so with the same BuildKit and revision the same files give the same digest.
#
# Needs Docker with buildx and the containerd image store (multi-platform builds).
#
# Usage: scripts/release-image.sh VERSION DIST OUT
#   IMAGE     image name (default ghcr.io/graphviz-pure/graphviz-pure)
#   REVISION  commit recorded in the labels (default: HEAD)
set -euo pipefail

version=$1 dist=$2 out=$3
root=$(cd "$(dirname "$0")/.." && pwd)
image=${IMAGE:-ghcr.io/graphviz-pure/graphviz-pure}
revision=${REVISION:-$(git -C "$root" rev-parse HEAD)}
SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git -C "$root" log -1 --format=%ct)}
export SOURCE_DATE_EPOCH
created=$(date -u -d "@$SOURCE_DATE_EPOCH" +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u -r "$SOURCE_DATE_EPOCH" +%Y-%m-%dT%H:%M:%SZ)

ctx=$(mktemp -d)
trap 'rm -rf "$ctx"' EXIT
for arch in amd64:x86_64 arm64:aarch64; do
  name=graphviz-pure-$version-${arch#*:}-unknown-linux-musl
  mkdir "$ctx/${arch%%:*}"
  tar -xzf "$dist/$name.tar.gz" -C "$ctx/${arch%%:*}" --strip-components=1 "$name/graphviz-pure"
done
# The amd64 entry point is the baseline launcher; choose v3 only on capable hosts.
mkdir "$ctx/amd64/v3" "$ctx/amd64/compat"
mv "$ctx/amd64/graphviz-pure" "$ctx/amd64/v3/graphviz-pure"
name=graphviz-pure-$version-x86_64-unknown-linux-musl
# Rename explicitly: keep release archive binary bytes intact.
tar -xOf "$dist/$name.tar.gz" "$name/graphviz-pure-launcher" > "$ctx/amd64/graphviz-pure"
tar -xOf "$dist/$name-compat.tar.gz" "$name-compat/graphviz-pure" > "$ctx/amd64/compat/graphviz-pure"
chmod 755 "$ctx/amd64/graphviz-pure" "$ctx/amd64/compat/graphviz-pure"
# The licence files are the same in both archives.
name=graphviz-pure-$version-x86_64-unknown-linux-musl
mkdir "$ctx/doc"
tar -xzf "$dist/$name.tar.gz" -C "$ctx/doc" --strip-components=1 \
  "$name/LICENSE" "$name/NOTICE" "$name/THIRD-PARTY-LICENSES.md" "$name/LICENSES"

# The labels are repeated as index annotations: a registry shows a multi-platform image's
# description, source and licence from its index. No buildx provenance or SBOM: the release
# workflow attests the pushed digest with Sigstore instead, and buildx's attestation manifests
# would add "unknown/unknown" entries to the index.
annotations=()
for kv in "title=graphviz-pure" \
  "description=A pure-Rust port of Graphviz with the same output on every platform: a drop-in dot command line" \
  "source=https://github.com/graphviz-pure/graphviz-pure" "url=https://graphviz-pure.github.io/" \
  "licenses=EPL-2.0" "version=$version" "revision=$revision" "created=$created"; do
  annotations+=(--annotation "index:org.opencontainers.image.$kv")
done

mkdir -p "$out"
docker buildx build \
  --platform linux/amd64,linux/arm64 \
  --file "$root/scripts/release-image.Dockerfile" \
  --build-arg "VERSION=$version" --build-arg "REVISION=$revision" --build-arg "CREATED=$created" \
  "${annotations[@]}" \
  --provenance=false --sbom=false \
  --output "type=oci,dest=$out/graphviz-pure-image.tar,name=$image:$version,rewrite-timestamp=true" \
  "$ctx"
tar -xOf "$out/graphviz-pure-image.tar" index.json \
  | python3 -c 'import json, sys; print(json.load(sys.stdin)["manifests"][0]["digest"])' \
  > "$out/graphviz-pure-image.digest"
echo "$image:$version $(cat "$out/graphviz-pure-image.digest")"
