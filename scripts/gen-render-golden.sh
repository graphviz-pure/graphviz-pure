#!/usr/bin/env bash
# Regenerates the render-fidelity corpus under tests/golden/render/ from C Graphviz 16.1.0,
# for the inputs in tests/wild, tests/fixtures and tests/render (renderer feature tests).
#
# Rendering is isolated from layout: C lays every corpus graph out once
# (`dot -Tdot`, honouring any `layout` attribute), and the result is re-read in
# no-layout mode (`neato -n2`), which keeps the node, edge, cluster and label
# positions from the input. The port renders the same laid-out file with
# `graphviz-pure -Kneato -n2`, so the two outputs differ only where the renderers do.
#
#   tests/golden/render/laid/STEM.dot     C layout with any `layout` attribute blanked
#   tests/golden/render/FORMAT/STEM.EXT   C `neato -n2 -TFORMAT` of the laid-out file
#
# As in scripts/gen-golden.sh, C computes with the port's math functions
# (scripts/dot-portable-libm.sh; build graphviz-pure-portable-libm first, C_DOT overrides it)
# and uses the built-in font metric tables (no pango),
# like the port. GVBINDIR must name a plugin directory holding only the core, dot
# and neato plugins; the default is ~/src/gv-ref/lutplugins.
#
# The directories are gitignored: the script ends by packing them into
# tests/golden/render.tar.xz (scripts/pack-render-golden.py), which is what git tracks.
#
# Usage: scripts/gen-render-golden.sh [GVBINDIR]
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
C_DOT="${C_DOT:-$REPO_ROOT/scripts/dot-portable-libm.sh}"
export GVBINDIR="${1:-$HOME/src/gv-ref/lutplugins}"
OUT="$REPO_ROOT/tests/golden/render"
FORMATS="svg svg_inline cmapx json json0 xdot dot plain plain-ext"
JSON_LIMIT=100000

version="$("$C_DOT" -V 2>&1 || true)"
case "$version" in
  *"version 16.1.0"*) ;;
  *) echo "need Graphviz 16.1.0 as $C_DOT, found: $version" >&2; exit 1 ;;
esac

# Keep the ratchet baseline (baseline.portable.json); rebuild everything else.
find "$OUT" -mindepth 1 -maxdepth 1 -type d -exec rm -rf {} + 2>/dev/null || true
mkdir -p "$OUT/laid"
for fmt in $FORMATS; do mkdir -p "$OUT/$fmt"; done

count=0
for f in "$REPO_ROOT"/tests/wild/*.gv "$REPO_ROOT"/tests/fixtures/*.gv "$REPO_ROOT"/tests/render/*.gv; do
  stem="$(basename "$f" .gv)"
  laid="$OUT/laid/$stem.dot"
  # Rewrite any `layout` attribute to neato: it overrides the -K engine, and -n2
  # applies only to neato.
  "$C_DOT" -Tdot "$f" 2>/dev/null | LC_ALL=C sed -E 's/(^|[[:space:],[])layout=[A-Za-z0-9_]+/\1layout=neato/' > "$laid" || true
  if [ ! -s "$laid" ]; then
    rm -f "$laid"
    continue
  fi
  for fmt in $FORMATS; do
    "$C_DOT" -Kneato -n2 -T"$fmt" "$laid" > "$OUT/$fmt/$stem.$fmt" 2>/dev/null || true
    [ -s "$OUT/$fmt/$stem.$fmt" ] || rm -f "$OUT/$fmt/$stem.$fmt"
  done
  # Keep the corpus small: -Tjson repeats the xdot draw operations verbosely, so
  # drop the JSON references of the few largest graphs.
  json="$OUT/json/$stem.json"
  if [ -f "$json" ] && [ "$(wc -c < "$json")" -gt "$JSON_LIMIT" ]; then rm -f "$json"; fi
  count=$((count + 1))
done
echo "wrote $count laid-out inputs and their reference renderings to $OUT"
for d in "$OUT"/*/; do
  printf '%-10s %4d files %9d bytes\n' "$(basename "$d")" "$(ls "$d" | wc -l)" "$(cat "$d"* | wc -c)"
done
# The references are tracked compressed; gv-diff render unpacks the archive (tests/golden/README.md).
python3 "$REPO_ROOT/scripts/pack-render-golden.py"
