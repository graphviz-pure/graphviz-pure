#!/usr/bin/env bash
# Extract Rust functions to Lean with Charon and Aeneas, into proofs/GvpProofs/Extracted/<Name>/.
#
# Usage:
#   scripts/proofs/extract.sh <crate-dir> <Name> [charon filter args...]
#   scripts/proofs/extract.sh --check
#
#   <crate-dir>   a workspace crate, e.g. crates/graphviz-pure-layout
#   <Name>        the Lean module directory and namespace, GvpProofs.Extracted.<Name>
#   filter args   Charon's item filters, passed to `charon cargo` unchanged, e.g.
#                 --start-from crate::circogen::blockpath::index_prefix
#                 (also --include, --opaque, --exclude; see `charon cargo --help`)
#
# The first form runs `charon cargo --preset=aeneas` in the crate, then
# `aeneas -backend lean -split-files`, and writes Types.lean and Funs.lean (plus any other file
# Aeneas generates) under proofs/GvpProofs/Extracted/<Name>/. It records the command in
# `extract.args` there, so --check can replay it. Aeneas lists the standard-library functions it
# has no model for in *External_Template.lean. The first extraction copies that template to
# *External.lean, which is committed and hand-written: replace each `axiom` with a definition.
# Later runs leave *External.lean alone and print the template's names when they change.
# Aeneas prints f32/f64 as F32/F64 but defines neither, so each generated file that mentions them
# also imports GvpProofs.Extracted.Floats (Lean's Float and Float32).
# If Aeneas rejects any function, nothing under proofs/ changes and the script exits 1. The
# partial output stays in the scratch directory, whose path is printed.
#
# --check re-extracts every proofs/GvpProofs/Extracted/*/extract.args into a scratch directory
# and fails if a generated file differs from the committed one. CI does not run extraction (the
# toolchain is too heavy), so run --check before opening a pull request that touches extracted
# Rust or proofs/GvpProofs/Extracted/.
#
# Toolchain: GVP_LEAN_HOME (default /jobs/work/gvp-lean) holds the Aeneas checkout built as in
# docs/research/lean-experiment/README.md; AENEAS_HOME overrides the checkout. The Aeneas commit
# must equal the `aeneas` rev in proofs/lakefile.toml. Cargo runs through cargo-slot.sh when it
# exists (the shared machine), with its target directory under /jobs/cargo-targets.
set -euo pipefail

root=$(git rev-parse --show-toplevel)
: "${GVP_LEAN_HOME:=/jobs/work/gvp-lean}"
: "${AENEAS_HOME:=$GVP_LEAN_HOME/aeneas}"
charon="$AENEAS_HOME/charon/bin/charon"
aeneas="$AENEAS_HOME/bin/aeneas"
extracted="$root/proofs/GvpProofs/Extracted"
wt=$(basename "$root")
scratch_root="${GVP_EXTRACT_SCRATCH:-$GVP_LEAN_HOME/extract/$wt}"
export CHARON_CACHE_DIR="${CHARON_CACHE_DIR:-$GVP_LEAN_HOME/charon-cache}"
if [ -z "${CARGO_TARGET_DIR:-}" ]; then
  if [ -d /jobs/cargo-targets ]; then
    export CARGO_TARGET_DIR="/jobs/cargo-targets/$wt-charon"
  else
    export CARGO_TARGET_DIR="$scratch_root/target"
  fi
fi

usage() {
  sed -n '3,6p' "$0" | sed 's/^# \{0,1\}//' >&2
  exit 64
}

check_toolchain() {
  for bin in "$charon" "$aeneas"; do
    [ -x "$bin" ] || { echo "extract.sh: $bin not found; see docs/research/lean-experiment/README.md" >&2; exit 69; }
  done
  local want have
  want=$(sed -n '/^name = "aeneas"/,/^\[/s/^rev = "\([0-9a-f]*\)"/\1/p' "$root/proofs/lakefile.toml")
  have=$("$aeneas" -version | awk '{print $2}')
  if [ -z "$want" ] || [ "${want#"$have"}" = "$want" ]; then
    echo "extract.sh: the Aeneas binary is commit $have, proofs/lakefile.toml pins ${want:-nothing}" >&2
    exit 69
  fi
}

# extract_to <out-root> <crate-dir> <Name> [filters...]: generate into <out-root>/GvpProofs/Extracted/<Name>.
extract_to() {
  local out=$1 crate=$2 name=$3
  shift 3
  local llbc="$out/$name.llbc"
  mkdir -p "$out"
  local runner=(nice -n 15)
  [ -x /usr/local/bin/cargo-slot.sh ] && runner=(/usr/local/bin/cargo-slot.sh)
  echo "extract.sh: charon cargo in $crate ($*)" >&2
  (cd "$root/$crate" && "${runner[@]}" "$charon" cargo --preset=aeneas --dest-file "$llbc" "$@") || {
    echo "extract.sh: Charon failed on $crate" >&2
    return 1
  }
  echo "extract.sh: aeneas -backend lean -> GvpProofs/Extracted/$name" >&2
  # -sequential: Aeneas otherwise starts one OCaml domain per CPU.
  nice -n 15 "$aeneas" -backend lean -split-files -no-progress-bar -sequential \
    -dest "$out" -subdir "GvpProofs/Extracted/$name" -namespace "GvpProofs.Extracted.$name" "$llbc" || {
    echo "extract.sh: Aeneas rejected part of $crate; partial output in $out/GvpProofs/Extracted/$name" >&2
    return 1
  }
  # Aeneas prints f32/f64 as F32/F64 without defining them (GvpProofs/Extracted/Floats.lean).
  local f
  for f in "$out/GvpProofs/Extracted/$name"/*.lean; do
    if grep -qE '\bF(32|64)\b' "$f"; then
      sed -i '0,/^public import Aeneas$/s//public import Aeneas\npublic import GvpProofs.Extracted.Floats/' "$f"
    fi
  done
}

# The generated files, the hand-written *External.lean and the templates excluded.
generated() {
  (cd "$1" && find . -maxdepth 1 -name '*.lean' ! -name '*External.lean' ! -name '*_Template.lean' | sort)
}

# The Rust items a Lean file models: the strings of its @[rust_fun "..."]-style attributes.
rust_names() {
  tr '\n' ' ' < "$1" | grep -oE '@\[rust_[a-z]+ +"[^"]*"' | sed 's/^@\[rust_[a-z]* *//' | sort || true
}

valid_name() {
  [[ "$1" =~ ^[A-Z][A-Za-z0-9]*$ ]] || { echo "extract.sh: Name must be UpperCamelCase, got '$1'" >&2; exit 64; }
}

if [ "${1:-}" = "--check" ]; then
  [ "$#" -eq 1 ] || usage
  check_toolchain
  status=0
  shopt -s nullglob
  for args in "$extracted"/*/extract.args; do
    dir=$(dirname "$args")
    name=$(basename "$dir")
    mapfile -t line < "$args"
    tmp="$scratch_root/check/$name"
    rm -rf "$tmp"
    if ! extract_to "$tmp" "${line[@]:0:1}" "$name" "${line[@]:1}"; then
      status=1
      continue
    fi
    new="$tmp/GvpProofs/Extracted/$name"
    if ! diff <(generated "$dir") <(generated "$new") >/dev/null; then
      echo "extract.sh: $name: the set of generated files changed" >&2
      diff <(generated "$dir") <(generated "$new") >&2 || true
      status=1
    fi
    while read -r f; do
      [ -f "$new/$f" ] || continue
      if ! diff -u "$dir/$f" "$new/$f"; then
        echo "extract.sh: $name/${f#./} is stale; rerun scripts/proofs/extract.sh $(tr '\n' ' ' < "$args")" >&2
        status=1
      fi
    done < <(generated "$dir")
    for t in "$new"/*_Template.lean; do
      h="$dir/$(basename "${t%_Template.lean}").lean"
      [ -f "$h" ] || { echo "extract.sh: $name: missing $(basename "$h") (template $(basename "$t"))" >&2; status=1; }
    done
  done
  [ "$status" -eq 0 ] && echo "extract.sh: every extraction is current"
  exit "$status"
fi

[ "$#" -ge 2 ] || usage
crate=${1%/}
name=$2
shift 2
valid_name "$name"
[ -f "$root/$crate/Cargo.toml" ] || { echo "extract.sh: $crate has no Cargo.toml" >&2; exit 64; }
check_toolchain
tmp="$scratch_root/run/$name"
rm -rf "$tmp"
extract_to "$tmp" "$crate" "$name" "$@" || exit 1
new="$tmp/GvpProofs/Extracted/$name"
dest="$extracted/$name"
mkdir -p "$dest"
# Replace the generated files; a file Aeneas no longer produces is removed.
while read -r f; do rm -f "$dest/$f"; done < <(generated "$dest")
while read -r f; do cp "$new/$f" "$dest/$f"; done < <(generated "$new")
shopt -s nullglob
for t in "$new"/*_Template.lean; do
  base=$(basename "${t%_Template.lean}")
  h="$dest/$base.lean"
  if [ -f "$h" ]; then
    names=$(rust_names "$t")
    have=$(rust_names "$h")
    if [ "$names" != "$have" ]; then
      echo "extract.sh: $name/$base.lean no longer matches Aeneas's template; compare with $t" >&2
    fi
  else
    {
      echo "-- Hand-written models of the external functions of $name, from Aeneas's template."
      echo "-- Replace every axiom with a definition that matches the Rust standard library."
      cat "$t"
    } > "$h"
    echo "extract.sh: created $name/$base.lean from the template; replace its axioms" >&2
  fi
done
printf '%s\n' "$crate" "$@" > "$dest/extract.args"
echo "extract.sh: wrote proofs/GvpProofs/Extracted/$name/"
