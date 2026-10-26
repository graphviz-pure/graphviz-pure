#!/usr/bin/env bash
# Runs C Graphviz `dot` with graphviz-pure's portable math functions in place of the system libm.
#
# Every build of the port computes with the same portable math functions
# (`graphviz_pure_core::math::portable`), never a platform libm. This script gives C the same
# functions, by loading crates/graphviz-pure-portable-libm into dot, so that the two can be
# compared with the math library taken out of the equation. It is the C reference of the corpus
# checks (`gv-diff e2e`, `scripts/gen-golden.sh`):
#
#   cargo build --release -p graphviz-pure-portable-libm
#   GVBINDIR=~/src/gv-ref/lutplugins cargo run --release -p graphviz-pure-diff -- e2e --generate --runs 5
#   cargo run --release -p graphviz-pure-diff -- e2e
#
# On macOS the library is loaded with DYLD_INSERT_LIBRARIES and interposes libm; on Linux,
# with LD_PRELOAD (scripts/c-oracle-linux/). The variable is set here rather than inherited
# because macOS strips DYLD_* variables when it starts a protected program such as /bin/bash.
# GV_PORTABLE_LIBM overrides the library's path.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# C must also measure text as the port does, with its built-in font metric tables rather than a
# pango textlayout plugin, so GVBINDIR has to name the pango-free plugin directory of the
# reference setup: one holding only the core, dot and neato layout plugins, with the config file
# `dot -c` writes there and no textlayout entry in it (docs/porting/libm.md; the directory
# scripts/gen-golden.sh builds). Unset — or a full installation's plugin directory — and pango
# measures C's text, so every comparison silently differs by millipoints (hidden #455).
GV_CONFIG="$(find "${GVBINDIR:-/no-such-dir}" -maxdepth 1 -name 'config[0-9]' -print 2>/dev/null | head -n 1 || true)"
if [ -z "$GV_CONFIG" ] || grep -q textlayout "$GV_CONFIG"; then
  echo "GVBINDIR='${GVBINDIR:-unset}' is not the pango-free C plugin directory of the reference" \
    "setup (only the core, dot and neato layout plugins, with the config file dot -c writes and no" \
    "textlayout plugin in it; docs/porting/libm.md): with a default configuration pango measures" \
    "C's text, and every comparison silently differs by millipoints" >&2
  exit 2
fi

TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
case "$(uname -s)" in
  Darwin) LIB="${GV_PORTABLE_LIBM:-$TARGET/release/libgraphviz_pure_portable_libm.dylib}"; VAR=DYLD_INSERT_LIBRARIES ;;
  *)      LIB="${GV_PORTABLE_LIBM:-$TARGET/release/libgraphviz_pure_portable_libm.so}"; VAR=LD_PRELOAD ;;
esac
[ -f "$LIB" ] || { echo "missing $LIB; run cargo build --release -p graphviz-pure-portable-libm" >&2; exit 2; }
env "$VAR=$LIB" dot "$@"
