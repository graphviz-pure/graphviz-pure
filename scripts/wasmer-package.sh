#!/usr/bin/env bash
# Writes the Wasmer package directory for a release: the wasm32-wasip1 release archive's module
# and licence files, packaging/wasmer/README.md and packaging/wasmer/wasmer.toml with the
# archive's version. release.yml builds it from the archive the attest job signed, so the
# published module is the attested one byte for byte.
#
# Usage: scripts/wasmer-package.sh ARCHIVE OUT
#   ARCHIVE  graphviz-pure-VERSION-wasm32-wasip1.tar.gz (scripts/release-package.py)
#   OUT      a directory that does not exist yet
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
archive=${1:?usage: scripts/wasmer-package.sh ARCHIVE OUT}
out=${2:?usage: scripts/wasmer-package.sh ARCHIVE OUT}
name=$(basename "$archive" .tar.gz)
version=${name#graphviz-pure-}
version=${version%-wasm32-wasip1}
[[ "$name" == "graphviz-pure-$version-wasm32-wasip1" && "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] ||
  { echo "error: $archive is not graphviz-pure-VERSION-wasm32-wasip1.tar.gz" >&2; exit 1; }
[ ! -e "$out" ] || { echo "error: $out exists" >&2; exit 1; }
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
tar -xzf "$archive" -C "$work"
src=$work/$name
mkdir -p "$out/doc"
cp "$src/graphviz-pure.wasm" "$src/LICENSE" "$out/"
cp -R "$src/NOTICE" "$src/THIRD-PARTY-LICENSES.md" "$src/LICENSE" "$src/LICENSES" "$out/doc/"
cp "$root/packaging/wasmer/README.md" "$out/"
sed "s/@VERSION@/$version/" "$root/packaging/wasmer/wasmer.toml" > "$out/wasmer.toml"
grep -q '@VERSION@' "$out/wasmer.toml" && { echo "error: version not filled in" >&2; exit 1; }
echo "$out"
