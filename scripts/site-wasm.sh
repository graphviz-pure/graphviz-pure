#!/usr/bin/env bash
# Builds the WASM package the site's live checks load, into site/public/wasm/.
# Same build as web/package.json `build:wasm` (opt-level z, fat LTO, wasm-bindgen --target web,
# then wasm-opt when it is installed). wasm-bindgen-cli must match the wasm-bindgen crate in
# Cargo.lock exactly.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
OUT="site/public/wasm"
want=$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' Cargo.lock)
have=$(wasm-bindgen --version | awk '{print $2}')
[ "$want" = "$have" ] || { echo "wasm-bindgen-cli $have, Cargo.lock wants $want" >&2; exit 1; }

CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS='-C opt-level=z' CARGO_PROFILE_RELEASE_LTO=fat \
  cargo build --release --target wasm32-unknown-unknown -p graphviz-pure-wasm
rm -rf "$OUT"
wasm-bindgen --target web --remove-name-section --remove-producers-section --out-dir "$OUT" target/wasm32-unknown-unknown/release/graphviz_pure_wasm.wasm
node "$ROOT/scripts/export-graphviz-error.mjs" "$OUT/graphviz_pure_wasm.js"
# The generated .d.ts types JsValue returns as `any`. The package ships the hand-checked one.
cp "$ROOT/web/graphviz_pure_wasm.d.ts" "$OUT/graphviz_pure_wasm.d.ts"
if command -v wasm-opt >/dev/null; then
  wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
    --enable-mutable-globals "$OUT/graphviz_pure_wasm_bg.wasm" -o "$OUT/graphviz_pure_wasm_bg.wasm"
fi
git rev-parse --short HEAD > "$OUT/COMMIT"
ls -l "$OUT"
