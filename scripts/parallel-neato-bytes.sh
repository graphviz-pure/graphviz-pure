#!/usr/bin/env bash
# Render neato graphs at one thread and at four, and require the same bytes.
#
# The binary must be built with the layout crate's `parallel` feature.
# GRAPHVIZ_PURE_THREADS=1 takes the sequential mat-vec. =4 splits a graph once
# it has at least 1300 nodes. tests/neato is below that, so this script also
# generates one 1400-node graph with four pinned nodes.
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
