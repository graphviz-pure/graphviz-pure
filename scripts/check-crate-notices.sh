#!/usr/bin/env bash
# Packages the crates that the release publishes (publish unset, as in scripts/crates-to-publish.py)
# and checks their licence and attribution files with scripts/check-package-notices.py.
# --no-verify: nothing is compiled. CI runs it on every change and the release workflow before
# anything is published.
set -euo pipefail
cd "$(dirname "$0")/.."
# One line per publishable crate: NAME PATH-OF-ITS-.crate
package_list() {
  cargo metadata --manifest-path "$1" --locked --no-deps --format-version 1 | python3 -c '
import json, os, sys
m = json.load(sys.stdin)
for p in m["packages"]:
    if p["publish"] is None:
        crate = "{}-{}.crate".format(p["name"], p["version"])
        print(p["name"], os.path.join(m["target_directory"], "package", crate))
'
}
list=$(package_list Cargo.toml)
vendor_list=$(package_list vendor/Cargo.toml)
test -n "$list" || { echo "error: no publishable crates in the main workspace" >&2; exit 1; }
test -n "$vendor_list" || { echo "error: no publishable crates in the vendor workspace" >&2; exit 1; }
args=() vendor_args=() crates=()
while read -r name crate; do
  args+=(-p "$name")
  crates+=("$crate")
done <<< "$list"
while read -r name crate; do
  vendor_args+=(-p "$name")
  crates+=("$crate")
done <<< "$vendor_list"
# --allow-dirty: the check reads the working tree, so it also works before a commit; nothing
# packaged here is published (CI and the release check out a clean tree anyway).
# Even --no-verify resolves registry dependencies. Before the first fork publish, satisfy only
# this check from the local sources. These command-line overrides are not packaged manifests.
local_forks=(
  --config 'patch.crates-io.graphviz-pure-tiny-skia.path="vendor/tiny-skia"'
  --config 'patch.crates-io.graphviz-pure-tiny-skia-path.path="vendor/tiny-skia-path"'
  --config 'patch.crates-io.graphviz-pure-kurbo.path="vendor/kurbo"'
  --config 'patch.crates-io.graphviz-pure-svgtypes.path="vendor/svgtypes"'
  --config 'patch.crates-io.graphviz-pure-skrifa.path="vendor/skrifa"'
  --config 'patch.crates-io.graphviz-pure-resvg.path="vendor/resvg"'
  --config 'patch.crates-io.graphviz-pure-usvg.path="vendor/usvg"'
  --config 'patch.crates-io.graphviz-pure-svg2pdf.path="vendor/svg2pdf"'
)
cargo package --locked --no-verify --allow-dirty "${local_forks[@]}" "${args[@]}"
cargo package --manifest-path vendor/Cargo.toml --locked --no-verify --allow-dirty "${local_forks[@]}" "${vendor_args[@]}"
python3 scripts/check-package-notices.py "${crates[@]}"
