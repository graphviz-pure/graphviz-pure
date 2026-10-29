#!/usr/bin/env bash
# Render sfdp graphs at one thread and at four, and require the same bytes.
#
# The binary must be built with the layout crate's `parallel` feature.
# GRAPHVIZ_PURE_THREADS=1 takes the sequential repulsive term. =4 precomputes
# it once a level has at least MIN_N nodes (5000). Fewer than four threads
# stay sequential, and the count is capped at the core count, so a machine
# with fewer than four cores would compare the sequential term with itself.
# tests/sfdp stays at or below 2000 nodes; this script also generates one
# 6000-node graph, the same shape as the bench graph sfdp_sparse_5000,
# strictly above that threshold.
#
# Usage: scripts/parallel-sfdp-bytes.sh /path/to/graphviz-pure
set -euo pipefail

cores=$(nproc)
if (( cores < 4 )); then
  echo "parallel-sfdp-bytes: nproc=${cores} is below 4. GRAPHVIZ_PURE_THREADS=4 would be capped at the core count, the repulsion gate would stay closed, and the script would compare the sequential term with itself." >&2
  exit 1
fi

bin=${1:?usage: parallel-sfdp-bytes.sh GRAPHVIZ-PURE}
repo=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

python3 -I "$repo/scripts/bench/synth.py" --only sfdp_bytes_6000 --out "$work/synth"
pinned="$work/synth/sfdp_bytes_6000.gv"
if ! grep -q 'n5999' "$pinned"; then
  echo "sfdp_bytes_6000 did not mention n5999" >&2
  exit 1
fi

shopt -s nullglob
files=("$repo"/tests/sfdp/*.gv)
if [[ ${#files[@]} -eq 0 ]]; then
  echo "no graphs in tests/sfdp" >&2
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
