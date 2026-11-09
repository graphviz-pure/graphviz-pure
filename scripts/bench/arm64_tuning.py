#!/usr/bin/env python3
"""Temporary measurement helper for the arm64 CPU-tuning workflow (#826).

  manifest BINARY --out M.json         hash SVG (whole corpus) and PNG (every 10th input) output
  compare REF.json OTHER.json ...      fail unless every manifest equals REF
  bench --set S --synth-dir D --out R.json LABEL=BINARY ...
                                       interleaved wall/CPU time (and perf counters if --perf)

Standard library only; runs on Linux, macOS and Windows.
"""
import argparse
import hashlib
import json
import math
import os
import pathlib
import platform
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]


def digest(result):
    h = hashlib.sha256()
    for part in (str(result.returncode).encode(), result.stdout, result.stderr):
        h.update(len(part).to_bytes(8, "little"))
        h.update(part)
    return h.hexdigest()


def cmd_manifest(args):
    os.chdir(ROOT)
    inputs = sorted(pathlib.Path("tests").rglob("*.gv"), key=lambda p: p.as_posix())
    out = {}
    start = time.monotonic()
    for index, path in enumerate(inputs):
        rel = path.as_posix()  # same argv (and diagnostics) on every OS
        formats = ["svg", "png"] if index % 10 == 0 else ["svg"]
        for fmt in formats:
            try:
                result = subprocess.run([args.binary, f"-T{fmt}", rel], capture_output=True, timeout=600)
            except subprocess.TimeoutExpired:
                sys.exit(f"{rel}: timed out")
            if result.returncode < 0 or result.returncode >= 0x80000000:
                sys.exit(f"{rel} -T{fmt}: crashed ({result.returncode})")
            out[f"{rel} {fmt}"] = digest(result)
    doc = {"binary": args.binary, "elapsed_seconds": time.monotonic() - start, "outputs": out}
    pathlib.Path(args.out).write_text(json.dumps(doc, indent=0, sort_keys=True) + "\n")
    print(f"{len(out)} outputs hashed in {doc['elapsed_seconds']:.1f}s")


def cmd_compare(args):
    ref = json.loads(pathlib.Path(args.reference).read_text())["outputs"]
    failed = False
    lines = []
    for other in args.others:
        got = json.loads(pathlib.Path(other).read_text())["outputs"]
        keys = sorted(set(ref) | set(got))
        bad = [k for k in keys if ref.get(k) != got.get(k)]
        same = len(keys) - len(bad)
        lines.append(f"{other}: {same} identical, {len(bad)} differ (of {len(keys)})")
        for k in bad[:50]:
            lines.append(f"  differs: {k}")
        failed |= bool(bad)
    text = "\n".join(lines)
    print(text)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as f:
            f.write(f"\n```\n{text}\n```\n")
    if failed:
        sys.exit(1)


def read_set(path):
    cases = []
    for line in pathlib.Path(path).read_text().splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        fields = line.split()
        formats = fields[1].split(",") if len(fields) > 1 else ["svg"]
        runs = int(fields[2]) if len(fields) > 2 else None
        for fmt in formats:
            cases.append((fields[0], fmt, runs))
    return cases


def run_once(argv, perf):
    """Wall seconds, CPU seconds (POSIX only) and perf counters (Linux with --perf)."""
    counters = {}
    perf_file = None
    if perf:
        fd, perf_file = tempfile.mkstemp(suffix=".perf")
        os.close(fd)
        argv = ["perf", "stat", "-x", ",", "-e", "instructions:u,cycles:u", "-o", perf_file, "--", *argv]
    start = time.perf_counter()
    proc = subprocess.Popen(argv, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    cpu = None
    if hasattr(os, "wait4"):
        _, status, usage = os.wait4(proc.pid, 0)
        proc.returncode = os.waitstatus_to_exitcode(status)
        cpu = usage.ru_utime + usage.ru_stime
    else:
        proc.wait()
    wall = time.perf_counter() - start
    if perf_file:
        for line in pathlib.Path(perf_file).read_text().splitlines():
            parts = line.split(",")
            if len(parts) > 2 and parts[0].strip().isdigit():
                counters[parts[2].split(":")[0]] = int(parts[0])
        os.unlink(perf_file)
    return {"wall": wall, "cpu": cpu, "exit": proc.returncode, **counters}


def geomean(values):
    values = [v for v in values if v and v > 0]
    return math.exp(sum(math.log(v) for v in values) / len(values)) if values else float("nan")


def median(values):
    values = sorted(values)
    n = len(values)
    return (values[n // 2] + values[(n - 1) // 2]) / 2


def machine():
    info = {"platform": platform.platform(), "machine": platform.machine(), "cpus": os.cpu_count()}
    try:
        if sys.platform == "darwin":
            info["cpu"] = subprocess.check_output(["sysctl", "-n", "machdep.cpu.brand_string"], text=True).strip()
        elif sys.platform == "win32":
            info["cpu"] = subprocess.check_output(
                ["powershell", "-NoProfile", "-Command", "(Get-CimInstance Win32_Processor).Name"], text=True).strip()
        else:
            info["cpu"] = subprocess.check_output(["lscpu"], text=True)
    except (OSError, subprocess.CalledProcessError) as e:
        info["cpu"] = f"unknown ({e})"
    return info


def cmd_bench(args):
    os.chdir(ROOT)
    builds = []
    for spec in args.builds:
        label, binary = spec.split("=", 1)
        builds.append((label, str(pathlib.Path(binary).resolve())))
    rows = []
    start = time.monotonic()
    for case_index, (inp, fmt, set_runs) in enumerate(read_set(args.set)):
        path = (pathlib.Path(args.synth_dir) / inp[6:]) if inp.startswith("synth/") else pathlib.Path(inp)
        runs = max(set_runs or args.runs, args.min_runs)
        samples = {label: [] for label, _ in builds}
        if set_runs is None:  # warm-up, as bench.py does for the fast cases
            for _, binary in builds:
                run_once([binary, f"-T{fmt}", str(path)], False)
        for r in range(runs):
            # Rotate the order every run so no build always goes first.
            shift = (r + case_index) % len(builds)
            for label, binary in builds[shift:] + builds[:shift]:
                samples[label].append(run_once([binary, f"-T{fmt}", str(path)], args.perf))
        row = {"input": inp, "format": fmt, "runs": runs, "samples": samples}
        rows.append(row)
        base = builds[0][0]
        msg = f"{inp} -T{fmt}:"
        for label, _ in builds:
            msg += f" {label} {median(s['wall'] for s in samples[label]) * 1000:.1f} ms"
            if args.perf and "cycles" in samples[label][0]:
                msg += f" ({median(s['cycles'] for s in samples[label]) / median(s['cycles'] for s in samples[base]):.4f} cyc)"
        print(msg, flush=True)
    doc = {"machine": machine(), "builds": dict(builds), "elapsed_seconds": time.monotonic() - start,
           "rows": rows}
    pathlib.Path(args.out).write_text(json.dumps(doc, indent=1) + "\n")
    summary = summarize(doc)
    print(summary)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as f:
            f.write("\n" + summary + "\n")


def engine_of(inp):
    name = inp.split("/")[1] if inp.startswith("tests/") else inp.split("/")[1].split("_")[0]
    return {"fixtures": "dot", "wild": "dot"}.get(name, name)


def summarize(doc):
    labels = list(doc["builds"])
    base = labels[0]
    metrics = ["wall", "cpu", "instructions", "cycles"]
    out = [f"Geomean of per-case medians, relative to `{base}` ({len(doc['rows'])} cases)", "",
           "| build | engine | " + " | ".join(metrics) + " |", "| :-- | :-- |" + " --: |" * len(metrics)]
    engines = sorted({engine_of(r["input"]) for r in doc["rows"]})
    for label in labels[1:]:
        for engine in ["all", *engines]:
            rows = [r for r in doc["rows"] if engine == "all" or engine_of(r["input"]) == engine]
            cells = []
            for m in metrics:
                ratios = []
                for r in rows:
                    a = [s.get(m) for s in r["samples"][label]]
                    b = [s.get(m) for s in r["samples"][base]]
                    if all(v is not None for v in a + b) and median(b) > 0:
                        ratios.append(median(a) / median(b))
                cells.append(f"{geomean(ratios):.4f}" if ratios else "—")
            out.append(f"| {label} | {engine} | " + " | ".join(cells) + " |")
    return "\n".join(out)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("manifest")
    p.add_argument("binary")
    p.add_argument("--out", required=True)
    p = sub.add_parser("compare")
    p.add_argument("reference")
    p.add_argument("others", nargs="+")
    p = sub.add_parser("bench")
    p.add_argument("--set", required=True)
    p.add_argument("--synth-dir", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--runs", type=int, default=5)
    p.add_argument("--min-runs", type=int, default=3)
    p.add_argument("--perf", action="store_true")
    p.add_argument("builds", nargs="+", help="LABEL=BINARY; the first is the baseline")
    p = sub.add_parser("summary")
    p.add_argument("result")
    args = parser.parse_args()
    if args.cmd == "summary":
        print(summarize(json.loads(pathlib.Path(args.result).read_text())))
    else:
        {"manifest": cmd_manifest, "compare": cmd_compare, "bench": cmd_bench}[args.cmd](args)


if __name__ == "__main__":
    main()
