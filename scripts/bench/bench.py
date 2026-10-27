#!/usr/bin/env python3
"""Wall time and peak memory of graphviz-pure (and optionally C Graphviz) on a benchmark set.

Standard library only. Each case is one input file and one output format; the program is run
`--runs` times and every run records wall time, user+system CPU time (`os.wait4`) and peak RSS
(`/usr/bin/time`'s %M; see run_once). The engine comes from the
input's own `layout=` attribute, as in the corpus checks, so the port and C both run as `dot`.

  python3 -I scripts/bench/synth.py --out target/bench/synth       # synthetic scaling graphs
  python3 -I scripts/bench/bench.py run --set scripts/bench/sets/full.tsv \\
      --c-dot /opt/graphviz-16.1.0/bin/dot --c-gvbindir /opt/graphviz-16.1.0/lutplugins \\
      --out target/bench/full.json
  python3 -I scripts/bench/bench.py run --set scripts/bench/sets/quick.tsv --out quick.json
  python3 -I scripts/bench/bench.py compare OLD.json NEW.json      # per-case ratios
  python3 -I scripts/bench/bench.py survey --out target/bench/survey.tsv   # every corpus input once

`run` writes the JSON file, a TSV next to it (same name, .tsv) and prints a Markdown summary.
See docs/perf/baseline.md for the method and the recorded baseline.
"""
import argparse
import concurrent.futures
import json
import os
import pathlib
import platform
import statistics
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
TIME = "/usr/bin/time" if os.path.exists("/usr/bin/time") else None
CORPORA = ["wild", "fixtures", "neato", "fdp", "sfdp", "circo", "twopi", "osage", "patchwork",
           "render"]


def run_once(argv, env=None, timeout=600):
    """Runs argv with stdout discarded; returns wall seconds, CPU seconds, peak RSS KiB, exit.

    Peak RSS comes from /usr/bin/time's %M. A child forked from Python starts with Python's own
    high-water mark (about 23 MB) and Linux keeps it across exec, so wait4's ru_maxrss would read
    23 MB for anything smaller; /usr/bin/time's own child starts from its small image instead.
    Wall time is measured here and CPU time comes from wait4 (both include /usr/bin/time's
    fork, well under a millisecond).
    """
    rss_file = f"/tmp/gvbench-rss-{os.getpid()}-{time.monotonic_ns()}"
    argv = [TIME, "-q", "-f", "%M", "-o", rss_file] + argv if TIME else argv
    start = time.perf_counter()
    # Its own process group, so a timeout kills the program and not only /usr/bin/time.
    proc = subprocess.Popen(argv, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env,
                            start_new_session=True)
    deadline = start + timeout
    while True:
        pid, status, usage = os.wait4(proc.pid, os.WNOHANG)
        if pid:
            break
        if time.perf_counter() > deadline:
            os.killpg(proc.pid, 9)
            pid, status, usage = os.wait4(proc.pid, 0)
            status = -1
            break
        time.sleep(0.0005)
    wall = time.perf_counter() - start
    proc.returncode = 0  # reaped by wait4; stop Popen from waiting again
    code = os.waitstatus_to_exitcode(status) if status != -1 else "timeout"
    rss = usage.ru_maxrss
    if TIME:
        try:
            rss = int(pathlib.Path(rss_file).read_text().split()[-1])
        except (OSError, ValueError, IndexError):
            rss = 0
        pathlib.Path(rss_file).unlink(missing_ok=True)
    return wall, usage.ru_utime + usage.ru_stime, rss, code


def c_env(gvbindir):
    env = dict(os.environ)
    if gvbindir:
        env["GVBINDIR"] = gvbindir
    return env


def read_set(path):
    """A set file: one case per line, `input [formats [runs]]`, whitespace-separated.

    formats is comma-separated (default svg); runs overrides --runs for a slow input. Paths are
    relative to the repository root; `synth/NAME.gv` means scripts/bench/synth.py's NAME in
    --synth-dir. # starts a comment.
    """
    cases = []
    for line in pathlib.Path(path).read_text().splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        fields = line.split("\t") if "\t" in line else line.split()
        inp = fields[0]
        formats = fields[1].split(",") if len(fields) > 1 else ["svg"]
        runs = int(fields[2]) if len(fields) > 2 else None
        for fmt in formats:
            cases.append((inp, fmt, runs))
    return cases


def load():
    try:
        with open("/proc/loadavg") as f:
            return f.read().split()[0]
    except OSError:
        return ""


def summarize(samples):
    walls = sorted(s[0] for s in samples)
    return {
        "wall_median": statistics.median(walls),
        "wall_min": walls[0],
        "wall_max": walls[-1],
        "cpu_median": statistics.median(s[1] for s in samples),
        "rss_kib_max": max(s[2] for s in samples),
        "exit": samples[0][3],
    }


def machine():
    model = ""
    try:
        for line in open("/proc/cpuinfo"):
            if line.startswith("model name"):
                model = line.split(":", 1)[1].strip()
                break
    except OSError:
        pass
    return {"cpu": model, "cpus": os.cpu_count(), "platform": platform.platform(),
            "python": platform.python_version()}


def cmd_run(args):
    cases = read_set(args.set)
    if args.only:
        cases = [c for c in cases if args.only in c[0]]
    port = str(pathlib.Path(args.port).resolve())
    version = subprocess.run([port, "-V"], capture_output=True, text=True).stderr.strip()
    results = []
    for inp, fmt, runs_line in cases:
        runs = min(runs_line or args.runs, args.runs)
        # A warm-up run (page cache, binary) unless the set marks the input as slow.
        warm = runs_line is None
        if inp.startswith("synth/"):
            path = pathlib.Path(args.synth_dir) / inp[len("synth/"):]
        else:
            path = (ROOT / inp) if not os.path.isabs(inp) else pathlib.Path(inp)
        if not path.exists():
            print(f"skip (missing): {inp}", file=sys.stderr)
            continue
        row = {"input": inp, "format": fmt, "bytes": path.stat().st_size, "runs": runs,
               "load_start": load()}
        argv = [port, f"-T{fmt}", str(path)]
        if warm:
            run_once(argv)
        row["port"] = summarize([run_once(argv) for _ in range(runs)])
        if args.c_dot and fmt in args.c_formats.split(","):
            cargv = [args.c_dot, f"-T{fmt}", str(path)]
            env = c_env(args.c_gvbindir)
            if warm:
                run_once(cargv, env)
            row["c"] = summarize([run_once(cargv, env) for _ in range(runs)])
            row["ratio_wall"] = row["port"]["wall_median"] / max(row["c"]["wall_median"], 1e-9)
            row["ratio_rss"] = row["port"]["rss_kib_max"] / max(row["c"]["rss_kib_max"], 1)
        row["load_end"] = load()
        results.append(row)
        print(line_for(row), file=sys.stderr, flush=True)
    doc = {"version": version, "machine": machine(), "runs": args.runs,
           "c_dot": args.c_dot, "c_gvbindir": args.c_gvbindir,
           "date": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "results": results}
    out = pathlib.Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(doc, indent=1) + "\n")
    write_tsv(results, out.with_suffix(".tsv"))
    print(markdown(results))


def line_for(row):
    p = row["port"]
    s = f"{row['input']} -T{row['format']}: port {p['wall_median']*1000:.1f} ms {p['rss_kib_max']} KiB"
    if "c" in row:
        s += f", C {row['c']['wall_median']*1000:.1f} ms {row['c']['rss_kib_max']} KiB"
    return s


def write_tsv(results, path):
    cols = ["input", "format", "bytes", "port_wall_ms", "port_min_ms", "port_max_ms",
            "port_cpu_ms", "port_rss_kib", "c_wall_ms", "c_cpu_ms", "c_rss_kib", "ratio_wall",
            "ratio_rss", "load_start"]
    lines = ["\t".join(cols)]
    for r in results:
        p, c = r["port"], r.get("c", {})
        vals = [r["input"], r["format"], r["bytes"], ms(p["wall_median"]), ms(p["wall_min"]),
                ms(p["wall_max"]), ms(p["cpu_median"]), p["rss_kib_max"],
                ms(c.get("wall_median")), ms(c.get("cpu_median")), c.get("rss_kib_max", ""),
                f"{r['ratio_wall']:.2f}" if "ratio_wall" in r else "",
                f"{r['ratio_rss']:.2f}" if "ratio_rss" in r else "", r["load_start"]]
        lines.append("\t".join(str(v) for v in vals))
    path.write_text("\n".join(lines) + "\n")


def ms(x):
    return "" if x is None else f"{x * 1000:.1f}"


def markdown(results):
    out = ["| input | -T | port ms (min–max) | port RSS MiB | C ms | C RSS MiB | port/C time | port/C RSS |",
           "| :-- | :-- | --: | --: | --: | --: | --: | --: |"]
    for r in results:
        p, c = r["port"], r.get("c")
        out.append("| {} | {} | {} ({}–{}) | {:.1f} | {} | {} | {} | {} |".format(
            r["input"].replace("tests/", ""), r["format"], ms(p["wall_median"]),
            ms(p["wall_min"]), ms(p["wall_max"]), p["rss_kib_max"] / 1024,
            ms(c["wall_median"]) if c else "", f"{c['rss_kib_max'] / 1024:.1f}" if c else "",
            f"{r['ratio_wall']:.2f}" if c else "", f"{r['ratio_rss']:.2f}" if c else ""))
    return "\n".join(out)


def cmd_compare(args):
    """Per-case ratios NEW/OLD of median wall time, CPU time and peak RSS; flags regressions."""
    old = {(r["input"], r["format"]): r for r in json.loads(pathlib.Path(args.old).read_text())["results"]}
    new = json.loads(pathlib.Path(args.new).read_text())["results"]
    print("| input | -T | metric | old | new | new/old |")
    print("| :-- | :-- | :-- | --: | --: | --: |")
    failed = []
    for r in new:
        o = old.get((r["input"], r["format"]))
        if not o:
            continue
        for key, scale in (("cpu_median", 1000), ("rss_kib_max", 1 / 1024)):
            a, b = o["port"][key], r["port"][key]
            ratio = b / a if a else 1.0
            mark = ""
            if ratio > args.tolerance:
                mark = " **regressed**"
                failed.append(r["input"])
            print(f"| {r['input']} | {r['format']} | {key} | {a*scale:.1f} | {b*scale:.1f} | {ratio:.2f}{mark} |")
    if failed and args.fail:
        print(f"{len(failed)} case(s) over tolerance {args.tolerance}", file=sys.stderr)
        sys.exit(1)


def cmd_survey(args):
    """Every corpus input once, in parallel: finds the slow and large inputs to put in a set."""
    port = str(pathlib.Path(args.port).resolve())
    inputs = []
    for corpus in CORPORA:
        inputs += sorted((ROOT / "tests" / corpus).glob("*.gv"))

    def one(path):
        r = run_once([port, f"-T{args.format}", str(path)], timeout=args.timeout)
        c = None
        if args.c_dot:
            c = run_once([args.c_dot, f"-T{args.format}", str(path)], c_env(args.c_gvbindir),
                         timeout=args.timeout)
        return path, r, c

    rows = []
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        for path, r, c in pool.map(one, inputs):
            rows.append((path.relative_to(ROOT), path.stat().st_size, r, c))
    rows.sort(key=lambda x: -x[2][0])
    lines = ["input\tbytes\tport_wall_ms\tport_rss_kib\tport_exit\tc_wall_ms\tc_rss_kib\tc_exit"]
    for p, size, r, c in rows:
        cc = [ms(c[0]), c[2], c[3]] if c else ["", "", ""]
        lines.append("\t".join(str(v) for v in [p, size, ms(r[0]), r[2], r[3], *cc]))
    pathlib.Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    pathlib.Path(args.out).write_text("\n".join(lines) + "\n")
    print("\n".join(lines[: args.top + 1]))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    default_port = str(ROOT / "target/release/graphviz-pure")

    r = sub.add_parser("run", help="time a benchmark set")
    r.add_argument("--set", required=True, help="set file (scripts/bench/sets/*.tsv)")
    r.add_argument("--port", default=default_port, help="graphviz-pure binary")
    r.add_argument("--runs", type=int, default=5, help="runs per case (a set line may lower it)")
    r.add_argument("--synth-dir", default=str(ROOT / "target/bench/synth"),
                   help="where scripts/bench/synth.py wrote the synthetic graphs")
    r.add_argument("--only", help="run only the cases whose input contains this string")
    r.add_argument("--c-dot", help="C dot binary to compare with")
    r.add_argument("--c-gvbindir", help="GVBINDIR for C (the pango-free plugin directory)")
    r.add_argument("--c-formats", default="svg,plain,dot", help="formats to time C on")
    r.add_argument("--out", required=True, help="JSON output; a .tsv is written next to it")
    r.set_defaults(func=cmd_run)

    c = sub.add_parser("compare", help="compare two run JSON files")
    c.add_argument("old")
    c.add_argument("new")
    c.add_argument("--tolerance", type=float, default=1.25, help="new/old ratio that fails")
    c.add_argument("--fail", action="store_true", help="exit 1 when a case is over tolerance")
    c.set_defaults(func=cmd_compare)

    s = sub.add_parser("survey", help="run every corpus input once")
    s.add_argument("--port", default=default_port)
    s.add_argument("--format", default="svg")
    s.add_argument("--c-dot")
    s.add_argument("--c-gvbindir")
    s.add_argument("--jobs", type=int, default=6)
    s.add_argument("--timeout", type=float, default=120)
    s.add_argument("--top", type=int, default=40)
    s.add_argument("--out", required=True)
    s.set_defaults(func=cmd_survey)

    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
