#!/usr/bin/env python3
"""The perf ratchet: instruction counts, heap allocations and peak heap on a benchmark set,
checked against a committed baseline (docs/perf/ratchet.md).

Standard library only. Every case runs once under Callgrind (instructions) and once as
examples/heap_counts.rs, the command line under dhat's counting allocator (allocations, bytes
allocated, peak heap). Both binaries are the static x86_64-unknown-linux-musl build, the one
releases ship for Linux, so the counts do not depend on the runner's glibc.

  python3 -I scripts/bench/ratchet.py                  # build, measure, check the baseline
  python3 -I scripts/bench/ratchet.py --update         # ... and rewrite the baseline
  python3 -I scripts/bench/ratchet.py --update --from target/ratchet/results.json
                                                       # the baseline from a measurement (a CI artifact)

The check fails when a case has more allocations than the baseline, more than 2% more
instructions, or more than 5% more peak heap; --accept reports the same and passes (the
`perf-regression-accepted` label in CI). Fewer pass: run --update to lock them in. Wall time is
not measured. The Markdown report goes to stdout and to $GITHUB_STEP_SUMMARY when it is set.
"""
import argparse
import concurrent.futures
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "bench"))
sys.dont_write_bytecode = True
from bench import read_set, run_once  # noqa: E402

TARGET = "x86_64-unknown-linux-musl"
TARGET_DIR = "target/ratchet"
BASELINE = "scripts/bench/ratchet-baseline.json"
SETS = ["scripts/bench/sets/quick.tsv", "scripts/bench/sets/ratchet-extra.tsv"]
# Every inputs path is relative to the repository root, where every case runs, so the arguments
# (and the allocations that hold them) are the same on every machine.
SYNTH_DIR = "target/bench/synth"
LAID_DIR = f"{TARGET_DIR}/laid"
# The release profile of .cargo/config.toml, pinned so that a change to the shipped profile
# moves the ratchet only when it is copied here (with --update).
PROFILE = {
    "CARGO_PROFILE_RELEASE_OPT_LEVEL": "3",
    "CARGO_PROFILE_RELEASE_LTO": "thin",
    "CARGO_PROFILE_RELEASE_CODEGEN_UNITS": "1",
    "CARGO_PROFILE_RELEASE_DEBUG": "false",
    "CARGO_PROFILE_RELEASE_PANIC": "unwind",
}
INSTRUCTIONS_TOLERANCE = 0.02
PEAK_TOLERANCE = 0.05


def cargo_build(args, wasm):
    env = dict(os.environ, **PROFILE)
    env.setdefault("CARGO_BUILD_JOBS", "6")
    base = ["cargo", "build", "--release", "--locked", "--target-dir", TARGET_DIR]
    subprocess.run(base + ["--target", TARGET, "-p", "graphviz-pure-cli", "--bin", "graphviz-pure",
                           "--example", "heap_counts"], cwd=ROOT, env=env, check=True)
    if wasm:
        subprocess.run(base + ["--target", "wasm32-unknown-unknown", "-p", "graphviz-pure-wasm"],
                       cwd=ROOT, env=env, check=True)


def binaries():
    """Relative paths, like the inputs: argv[0] is allocated too."""
    out = pathlib.Path(TARGET_DIR) / TARGET / "release"
    return out / "graphviz-pure", out / "examples" / "heap_counts"


def read_cases():
    """quick.tsv's `input formats` lines and ratchet-extra.tsv's `input format flags...` lines.

    `synth/NAME.gv` is scripts/bench/synth.py's graph; `laid/PATH` is PATH laid out by this
    build (`-Tdot`), the input of the cases that skip layout (`-Knop2`).
    """
    cases = []
    for inp, fmt, _ in read_set(ROOT / SETS[0]):
        cases.append((inp, fmt, []))
    for line in (ROOT / SETS[1]).read_text().splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            inp, fmt, *flags = line.split()
            cases.append((inp, fmt, flags))
    return cases


def case_id(inp, fmt, flags):
    return " ".join([inp, f"-T{fmt}"] + flags)


def input_path(inp):
    if inp.startswith("synth/"):
        return f"{SYNTH_DIR}/{inp[len('synth/'):]}"
    if inp.startswith("laid/"):
        return f"{LAID_DIR}/{inp[len('laid/'):]}"
    return inp


def prepare_inputs(cases, port):
    subprocess.run([sys.executable, "-I", "scripts/bench/synth.py", "--out", SYNTH_DIR], cwd=ROOT,
                   check=True, stdout=subprocess.DEVNULL)
    for inp, _, _ in cases:
        if inp.startswith("laid/"):
            src = inp[len("laid/"):]
            dst = ROOT / input_path(inp)
            dst.parent.mkdir(parents=True, exist_ok=True)
            with open(dst, "wb") as out:
                subprocess.run([str(port), "-Tdot", src], cwd=ROOT, stdout=out,
                               stderr=subprocess.DEVNULL, check=True)


def callgrind(valgrind, argv):
    """Instructions executed (Callgrind's `Collected`, which is Ir) and the exit status."""
    p = subprocess.run([valgrind, "--tool=callgrind", "--callgrind-out-file=/dev/null"] + argv,
                       cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True,
                       errors="replace")
    m = re.search(r"Collected : (\d+)", p.stderr)
    if not m:
        raise RuntimeError(f"no Callgrind total for {argv}:\n{p.stderr[-2000:]}")
    return int(m.group(1)), p.returncode


def heap(argv):
    p = subprocess.run(argv, cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                       text=True, errors="replace")
    for line in reversed(p.stderr.splitlines()):
        if line.startswith('{"heap_blocks"'):
            return json.loads(line)
    raise RuntimeError(f"no heap statistics for {argv}:\n{p.stderr[-2000:]}")


def peak_rss(argv):
    """Peak RSS in KiB of a native run (reported, not checked: it depends on the kernel)."""
    return run_once(argv)[2]


def measure_case(valgrind, port, counter, case):
    inp, fmt, flags = case
    args = [f"-T{fmt}"] + flags + [input_path(inp)]
    instructions, code = callgrind(valgrind, [str(port)] + args)
    row = {"instructions": instructions, "exit": code}
    row.update(heap([str(counter)] + args))
    row["rss_kib"] = peak_rss([str(port)] + args)
    return case_id(*case), row


def measure(args):
    if not args.no_build:
        cargo_build(args, args.wasm)
    port, counter = binaries()
    cases = read_cases()
    if args.only:
        cases = [c for c in cases if args.only in case_id(*c)]
    prepare_inputs(cases, port)
    results = {}
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        futures = [pool.submit(measure_case, args.valgrind, port, counter, c) for c in cases]
        for f in futures:
            cid, row = f.result()
            results[cid] = row
            print(f"{cid}: {row['instructions']:,} instructions, {row['heap_blocks']:,} "
                  f"allocations, peak heap {row['heap_peak_bytes']:,} B", file=sys.stderr)
    doc = {"target": TARGET, "cases": results}
    wasm = ROOT / TARGET_DIR / "wasm32-unknown-unknown/release/graphviz_pure_wasm.wasm"
    if args.wasm and wasm.exists():
        doc["wasm_bytes"] = wasm.stat().st_size
    return doc


def pct(new, old):
    return f"{(new - old) / old * 100:+.2f}%" if old else ""


def check(base, new, accept):
    """The Markdown report and the list of regressions."""
    failures, improved = [], []
    rows = []
    bcases = base.get("cases", {})
    ref = base.get("reference", {})
    c_ref = base.get("c_instructions", {})
    for cid, n in new["cases"].items():
        b = bcases.get(cid)
        if b is None:
            if n.get("exit", 0) != 0:
                failures.append(f"`{cid}`: exit status {n['exit']}")
            rows.append(f"| `{cid}` | {n['instructions']:,} | new | {n['heap_blocks']:,} | new | "
                        f"{n['heap_peak_bytes']:,} | new | {n['heap_bytes']:,} | | |")
            continue
        notes = []
        if n.get("exit", 0) != 0:
            notes.append(f"exit status {n['exit']}")
        if n["heap_blocks"] > b["heap_blocks"]:
            notes.append("allocations")
        if n["instructions"] > b["instructions"] * (1 + INSTRUCTIONS_TOLERANCE):
            notes.append("instructions")
        if n["heap_peak_bytes"] > b["heap_peak_bytes"] * (1 + PEAK_TOLERANCE):
            notes.append("peak heap")
        if notes:
            failures.append(f"`{cid}`: {', '.join(notes)}")
        elif (n["heap_blocks"] < b["heap_blocks"] or n["heap_peak_bytes"] < b["heap_peak_bytes"]
              or n["instructions"] < b["instructions"] * (1 - INSTRUCTIONS_TOLERANCE / 2)):
            improved.append(cid)
        c = c_ref.get(cid)
        r = ref.get("instructions", {}).get(cid)
        flag = f" **{', '.join(notes)}**" if notes else ""
        ins = n["instructions"]
        vs_c = f"{ins / c:.2f}" if c else ""
        vs_ref = f"{ins / r:.2f}" if r else ""
        rows.append(
            f"| `{cid}`{flag} | {ins:,} | {pct(ins, b['instructions'])} "
            f"| {n['heap_blocks']:,} | {n['heap_blocks'] - b['heap_blocks']:+,} "
            f"| {n['heap_peak_bytes']:,} | {pct(n['heap_peak_bytes'], b['heap_peak_bytes'])} "
            f"| {n['heap_bytes']:,} | {vs_c} | {vs_ref} |")
    missing = sorted(set(bcases) - set(new["cases"]))
    out = ["## Perf ratchet", ""]
    if failures:
        verdict = "accepted (`perf-regression-accepted`)" if accept else "**failed**"
        out += [f"Regressions, {verdict}:", ""] + [f"- {f}" for f in failures] + [""]
    else:
        out += ["No regression.", ""]
    if improved:
        out += [f"Improved: {len(improved)} case(s). Lock them in: `python3 -I "
                "scripts/bench/ratchet.py --update --from results.json` with this run's "
                "`perf-ratchet` artifact, and commit scripts/bench/ratchet-baseline.json.", ""]
    if missing:
        out += [f"Not measured (in the baseline): {', '.join(f'`{m}`' for m in missing)}", ""]
    refname = ref.get("name", "reference")
    out += [f"Gates: allocations exact, instructions +{INSTRUCTIONS_TOLERANCE:.0%}, peak heap "
            f"+{PEAK_TOLERANCE:.0%}. Δ against scripts/bench/ratchet-baseline.json; ×C and "
            f"×{refname} are instruction ratios against C 16.1.0 and {refname}.", "",
            f"| case | instructions | Δ | allocations | Δ | peak heap B | Δ | bytes allocated "
            f"| ×C | ×{refname} |",
            "| :-- | --: | --: | --: | --: | --: | --: | --: | --: | --: |"] + rows
    if "wasm_bytes" in new:
        w, bw = new["wasm_bytes"], base.get("wasm_bytes")
        out += ["", f"graphviz_pure_wasm.wasm (release, before wasm-bindgen): {w:,} B"
                + (f" ({pct(w, bw)} against the baseline's {bw:,} B)" if bw else "")]
    return "\n".join(out) + "\n", failures


def c_reference(args, base):
    """C's instructions for every case, under the same Callgrind (headline numbers only)."""
    port, _ = binaries()
    cases = read_cases()
    prepare_inputs(cases, port)
    env = dict(os.environ, GVBINDIR=args.c_gvbindir) if args.c_gvbindir else None
    out = {}

    def one(case):
        inp, fmt, flags = case
        argv = [args.valgrind, "--tool=callgrind", "--callgrind-out-file=/dev/null", args.c_dot,
                f"-T{fmt}"] + flags + [input_path(inp)]
        p = subprocess.run(argv, cwd=ROOT, env=env, stdout=subprocess.DEVNULL,
                           stderr=subprocess.PIPE, text=True, errors="replace")
        m = re.search(r"Collected : (\d+)", p.stderr)
        return case_id(*case), int(m.group(1)) if m and p.returncode == 0 else None

    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        for cid, n in pool.map(one, cases):
            if n:
                out[cid] = n
            print(f"C {cid}: {n}", file=sys.stderr)
    base["c_instructions"] = out


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--baseline", default=str(ROOT / BASELINE))
    ap.add_argument("--out", default=str(ROOT / TARGET_DIR / "results.json"),
                    help="where the measurement is written")
    ap.add_argument("--from", dest="source", help="check (or --update from) this measurement "
                    "instead of measuring")
    ap.add_argument("--update", action="store_true", help="rewrite the baseline's counts")
    ap.add_argument("--accept", action="store_true", help="report regressions but pass")
    ap.add_argument("--no-build", action="store_true", help="use the binaries already built")
    ap.add_argument("--wasm", action="store_true", help="also build and report the .wasm size")
    ap.add_argument("--only", help="only the cases whose id contains this string")
    ap.add_argument("--jobs", type=int, default=min(4, os.cpu_count() or 1))
    ap.add_argument("--valgrind", default=shutil.which("valgrind") or "valgrind")
    ap.add_argument("--set-reference", metavar="NAME",
                    help="with --update: also store these instruction counts as the reference "
                    "release NAME (the ×NAME column)")
    ap.add_argument("--c-dot", help="measure C dot's instructions into the baseline instead")
    ap.add_argument("--c-gvbindir", help="GVBINDIR for --c-dot")
    args = ap.parse_args()
    # Every case runs here (see SYNTH_DIR); run_once has no cwd argument.
    os.chdir(ROOT)

    bpath = pathlib.Path(args.baseline)
    base = json.loads(bpath.read_text()) if bpath.exists() else {"cases": {}}
    if args.c_dot:
        c_reference(args, base)
        bpath.write_text(json.dumps(base, indent=1, sort_keys=True) + "\n")
        return
    if args.source:
        new = json.loads(pathlib.Path(args.source).read_text())
    else:
        new = measure(args)
        out = pathlib.Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(new, indent=1, sort_keys=True) + "\n")
    report, failures = check(base, new, args.accept)
    print(report)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(report)
    if args.update:
        keep = {"heap_blocks", "heap_bytes", "heap_peak_bytes", "instructions"}
        rows = {cid: {k: v for k, v in row.items() if k in keep}
                for cid, row in new["cases"].items()}
        # --only measured a subset: keep the other cases' counts.
        base["cases"] = {**base["cases"], **rows} if args.only else rows
        base["target"] = new["target"]
        if "wasm_bytes" in new:
            base["wasm_bytes"] = new["wasm_bytes"]
        if args.set_reference:
            base["reference"] = {"name": args.set_reference, "instructions": {
                cid: row["instructions"] for cid, row in new["cases"].items()}}
        bpath.write_text(json.dumps(base, indent=1, sort_keys=True) + "\n")
        print(f"wrote {bpath}", file=sys.stderr)
        return
    if failures and not args.accept:
        sys.exit(1)


if __name__ == "__main__":
    main()
