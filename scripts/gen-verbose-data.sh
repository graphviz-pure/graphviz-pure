#!/usr/bin/env bash
# Writes crates/graphviz-pure-cli/tests/data/verbose/<engine>.txt: C Graphviz's `-v` stderr for
# the input below, with the port's math (scripts/dot-portable-libm.sh), normalised as the
# test `issue416_verbose_layout_messages_follow_c` normalises the port's: the plugin lines
# (#298's) are dropped, and the timestamps of `[Graphviz] file:line:` lines, the timings
# (`<n> sec`) and the uninitialised `value` of a non-prism `overlap:` line are masked.
#
#   cargo build --release -p graphviz-pure-portable-libm
#   GVBINDIR=~/src/gv-ref/lutplugins scripts/gen-verbose-data.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/crates/graphviz-pure-cli/tests/data/verbose"
mkdir -p "$OUT"
for e in dot neato fdp sfdp twopi osage patchwork; do
  echo 'digraph{a->b; b->c; a->c; d}' |
    "$ROOT/scripts/dot-portable-libm.sh" -v -K"$e" -Tplain 2>&1 >/dev/null |
    python3 -I -c '
import re, sys
skip = re.compile(r"^(\S+ - graphviz.* version |libdir = |Activated plugin library|Loading /|The plugin configuration file|\t\S|\t\twas successfully|Demand loading of plugins|    (render|layout|textlayout|device|loadimage)\t:|Using (render|device|layout|textlayout|loadimage): )")
for l in sys.stdin.read().split("\n"):
    if skip.match(l):
        continue
    l = re.sub(r"^\[Graphviz\] (\S+:\d+): \d{4}-\d\d-\d\d \d\d:\d\d:\d\d ", r"[Graphviz] \1: TS ", l)
    l = re.sub(r"-?\d+\.\d+(\s*)(secs?\b|sec\.)", r"T\1\2", l)
    m = re.match(r"^overlap: (.*) value (-?\d+) scaling (\S+)$", l)
    if m and not m.group(1).startswith("prism"):
        l = "overlap: %s value V scaling %s" % (m.group(1), m.group(3))
    print(l)
' | sed -e '$ { /^$/d; }' > "$OUT/$e.txt"
done
