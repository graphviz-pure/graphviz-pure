#!/usr/bin/env bash
# Render neato graphs at one thread and at four, and require the same bytes.
#
# The binary must be built with the layout crate's `parallel` feature.
# Neato's mat-vec is sequential at every thread count: four threads lost to
# one on musl PGO at 1300–2000, and a MIN_N of 1,000,000 (packed ~2 TB)
# made the workers unreachable, so they were removed. GRAPHVIZ_PURE_THREADS
# does not split this kernel. This script still requires =1 and =4 to match
# on tests/neato plus a 1400-node pinned graph (large sequential identity,
# not a parallel split). Sfdp's quadtree repulsion is the parallel path;
# see parallel-sfdp-bytes.sh.
#
# Usage: scripts/parallel-neato-bytes.sh /path/to/graphviz-pure
set -euo pipefail

bin=${1:?usage: parallel-neato-bytes.sh GRAPHVIZ-PURE}
repo=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

python3 -I "$repo/scripts/bench/synth.py" --only pinned_neato_1400 --out "$work/synth"
pinned="$work/synth/pinned_neato_1400.gv"
if ! grep -q 'n1399 \[' "$pinned"; then
  echo "pinned_neato_1400 did not declare n1399" >&2
  exit 1
fi

shopt -s nullglob
files=("$repo"/tests/neato/*.gv)
if [[ ${#files[@]} -eq 0 ]]; then
  echo "no graphs in tests/neato" >&2
  exit 1
fi
files+=("$pinned")

fail=0
for src in "${files[@]}"; do
  set +e
  GRAPHVIZ_PURE_THREADS=1 "$bin" -Tplain "$src" >"$work/one.out" 2>"$work/one.err"
  e1=$?
  GRAPHVIZ_PURE_THREADS=4 "$bin" -Tplain "$src" >"$work/four.out" 2>"$work/four.err"
  e2=$?
  set -e
  name=${src#"$repo"/}
  if [[ $name == "$src" ]]; then
    name=$(basename "$src")
  fi
  if [[ $e1 -ne $e2 ]] ||
    ! cmp -s "$work/one.out" "$work/four.out" ||
    ! cmp -s "$work/one.err" "$work/four.err"; then
    echo "differ: $name (exit $e1 vs $e2)" >&2
    fail=1
  fi
done

echo "compared ${#files[@]} graphs at GRAPHVIZ_PURE_THREADS=1 and =4"
[[ $fail -eq 0 ]]
