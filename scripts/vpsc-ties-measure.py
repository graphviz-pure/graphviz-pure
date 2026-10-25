#!/usr/bin/env python3
"""Measures C against the port on tests/vpsc-ties/ and the corpus's ipsep/vpsc cluster files.

For each input: C with the port's math (scripts/dot-portable-libm.sh) run RUNS times, the
port once, and, on macOS, C once more with scripts/c-bump-malloc.c interposed and address
randomisation off (through lldb), where objects are ordered by address in creation order.
Writes tests/vpsc-ties/outcomes.tsv: file, C's outcome (`fails`, `one` output or `varies`),
how many of the runs gave the port's output, and whether the bump-allocated C equals the
port. Runs one process at a time.

  cargo build --release -p graphviz-pure-cli -p graphviz-pure-portable-libm
  cc -O2 -dynamiclib -o target/c-bump-malloc.dylib scripts/c-bump-malloc.c
  GVBINDIR=~/src/gv-ref/lutplugins scripts/vpsc-ties-measure.py [RUNS]
"""
import os
import pathlib
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
RUNS = int(sys.argv[1]) if len(sys.argv) > 1 else 10
C_DOT = str(ROOT / "scripts" / "dot-portable-libm.sh")
PORT = str(ROOT / "target" / "release" / "graphviz-pure")
LIBM = ROOT / "target" / "release" / "libgraphviz_pure_portable_libm.dylib"
BUMP = ROOT / "target" / "c-bump-malloc.dylib"
# bump_c() runs dot directly (under lldb), so the pango-free check of scripts/dot-portable-libm.sh
# cannot guard it; require the same reference setup here.
GVBINDIR = os.environ.get("GVBINDIR") or sys.exit(
    "set GVBINDIR to the pango-free C plugin directory (docs/porting/libm.md); with a default"
    " configuration pango measures C's text")
# Runs ARGV in a process group of its own and kills the group on timeout (fuzz-crash.py).
SUPERVISE = r"""
$t = shift; $p = 0;
for my $s (qw(ALRM TERM INT HUP)) { $SIG{$s} = sub { kill 'KILL', -$p if $p; exit($_[0] eq 'ALRM' ? 142 : 143) } }
$p = fork; defined $p or exit 127;
if (!$p) { $SIG{$_} = 'DEFAULT' for qw(ALRM TERM INT HUP); setpgrp(0, 0); alarm $t + 2; exec @ARGV; exit 127 }
setpgrp($p, $p);
alarm $t; waitpid($p, 0);
exit($? & 127 ? 128 + ($? & 127) : $? >> 8);
"""


def run(argv):
    p = subprocess.run(["perl", "-e", SUPERVISE, "60"] + argv, capture_output=True)
    return p.returncode, p.stdout


def bump_c(path):
    """C with the bump allocator, no address randomisation, or None where unavailable."""
    if sys.platform != "darwin" or not BUMP.exists():
        return None
    with tempfile.NamedTemporaryFile(suffix=".plain") as out:
        env = f"DYLD_INSERT_LIBRARIES={BUMP}:{LIBM} GVBINDIR={GVBINDIR}"
        run(["lldb", "-b", "-o", "settings set target.disable-aslr true",
             "-o", f"settings set target.env-vars {env}",
             "-o", f"process launch -o {out.name} -e /dev/null -- -Tplain {path}",
             "/opt/homebrew/bin/dot"])
        data = pathlib.Path(out.name).read_bytes()
        return data or None


def inputs():
    files = sorted((ROOT / "tests" / "vpsc-ties").glob("*.gv"))
    for f in sorted((ROOT / "tests" / "neato").glob("*.gv")):
        t = f.read_text(errors="replace")
        if ("ipsep" in t or "vpsc" in t) and "cluster" in t:
            files.append(f)
    return files


def main():
    rows = ["file\tc\tport_in_runs\tbump_c_equals_port"]
    for f in inputs():
        outs = []
        for _ in range(RUNS):
            rc, o = run([C_DOT, "-Tplain", str(f)])
            outs.append(o if rc == 0 and o else None)
        port = run([PORT, "-Tplain", str(f)])[1]
        if None in outs:
            c, hits, bump = "fails", "-", "-"
        else:
            c = "one" if len(set(outs)) == 1 else "varies"
            hits = f"{outs.count(port)}/{RUNS}"
            b = bump_c(f)
            bump = "-" if b is None else ("yes" if b == port else "no")
        rows.append(f"{f.relative_to(ROOT)}\t{c}\t{hits}\t{bump}")
        print(rows[-1], flush=True)
    (ROOT / "tests" / "vpsc-ties" / "outcomes.tsv").write_text("\n".join(rows) + "\n")


if __name__ == "__main__":
    main()
