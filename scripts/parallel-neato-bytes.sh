#!/usr/bin/env bash
# Render every tests/neato graph twice and require the same bytes.
#
# The binary must be built with the layout crate's `parallel` feature.
# GRAPHVIZ_PURE_THREADS=1 takes the sequential mat-vec. =4 may split large
# graphs across threads. The corpus itself is below that split, so this check
# is the feature-on command line; the bit-identity of the split kernel is the
# layout crate's tests, which CI runs in the same job.
#
# Usage: scripts/parallel-neato-bytes.sh /path/to/graphviz-pure
set -euo pipefail

bin=${1:?usage: parallel-neato-bytes.sh GRAPHVIZ-PURE}
repo=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

shopt -s nullglob
files=("$repo"/tests/neato/*.gv)
if [[ ${#files[@]} -eq 0 ]]; then
  echo "no graphs in tests/neato" >&2
  exit 1
fi

fail=0
for src in "${files[@]}"; do
  set +e
  GRAPHVIZ_PURE_THREADS=1 "$bin" -Tplain "$src" >"$work/one.out" 2>"$work/one.err"
  e1=$?
  GRAPHVIZ_PURE_THREADS=4 "$bin" -Tplain "$src" >"$work/four.out" 2>"$work/four.err"
  e2=$?
  set -e
  if [[ $e1 -ne $e2 ]] ||
    ! cmp -s "$work/one.out" "$work/four.out" ||
    ! cmp -s "$work/one.err" "$work/four.err"; then
    echo "differ: ${src#"$repo"/} (exit $e1 vs $e2)" >&2
    fail=1
  fi
done

echo "compared ${#files[@]} graphs at GRAPHVIZ_PURE_THREADS=1 and =4"
[[ $fail -eq 0 ]]
