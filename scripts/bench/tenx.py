#!/usr/bin/env python3
"""The 1.x "10x faster than 1.0" headline (docs/perf/1.x-10x.md) from `bench.py run` JSON files.

Standard library only. BASE is the 1.0 run (with C timings, `--c-dot`); NEW is a later run of the
same set on the same machine. Without NEW it prints the baseline: the port's 1.0 time against C.

  python3 -I scripts/bench/tenx.py docs/perf/data/full-1.0.0.json            # baseline only
  python3 -I scripts/bench/tenx.py docs/perf/data/full-1.0.0.json new.json   # speedup vs 1.0
  python3 -I scripts/bench/tenx.py BASE.json NEW.json --parallel NEWPAR.json # + parallel column

Rules (docs/perf/1.x-10x.md, "The metric"):
- A case is one input and one `-T` format; its time is the median wall time.
- Speedup = 1.0 time / new time. Geometric means are taken per engine, per format and overall.
- Cases where 1.0 takes under `--floor` seconds (default 0.05) are process start-up, not layout:
  they are reported in their own "start-up" row and left out of the headline.
- "x C" = C time / new time, so above 1 means faster than C.
- Peak RSS: new / 1.0, geometric mean; above 1 is a memory regression.
"""

import argparse
import json
import math
import pathlib
import sys

ENGINES = ("dot", "neato", "fdp", "sfdp", "circo", "twopi", "osage", "patchwork")


def engine_of(inp):
    """Engine of a set input: tests/<engine>/..., synth/<engine>_N.gv, else dot (wild, fixtures)."""
    parts = pathlib.PurePosixPath(inp).parts
    if parts[0] == "synth":
        return parts[-1].split("_")[0]
    if len(parts) > 1 and parts[1] in ENGINES:
        return parts[1]
    return "dot"


def gmean(xs):
    xs = [x for x in xs if x and x > 0]
    return math.exp(sum(math.log(x) for x in xs) / len(xs)) if xs else None


def load(path):
    if not path:
        return None
    return {(r["input"], r["format"]): r for r in json.loads(pathlib.Path(path).read_text())["results"]}


def fmt(x, nd=2):
    return "" if x is None else f"{x:.{nd}f}"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("base", help="1.0 run JSON, with C timings")
    ap.add_argument("new", nargs="?", help="run JSON to compare with 1.0")
    ap.add_argument("--parallel", help="run JSON of a --features parallel build")
    ap.add_argument("--floor", type=float, default=0.05, help="start-up floor in seconds")
    ap.add_argument("--cases", action="store_true", help="also print every case")
    args = ap.parse_args()

    base, new, par = load(args.base), load(args.new), load(args.parallel)
    rows = []  # (group keys, base result, new result or None, parallel result or None)
    for key, b in base.items():
        if b["port"].get("exit") != 0:
            continue
        n = new.get(key) if new else None
        p = par.get(key) if par else None
        if new and (not n or n["port"].get("exit") != 0):
            continue
        rows.append((key, b, n, p))

    def stats(sel):
        sp, spc, xc, rss, base_xc = [], [], [], [], []
        for (_, b, n, p) in sel:
            bt = b["port"]["wall_median"]
            ct = (b.get("c") or {}).get("wall_median") if (b.get("c") or {}).get("exit") == 0 else None
            if ct:
                base_xc.append(ct / bt)
            if n:
                nt = n["port"]["wall_median"]
                sp.append(bt / nt)
                rss.append(n["port"]["rss_kib_max"] / b["port"]["rss_kib_max"])
                if ct:
                    xc.append(ct / nt)
                if p:
                    spc.append(bt / p["port"]["wall_median"])
        return len(sel), gmean(sp), gmean(spc), gmean(xc), gmean(rss), gmean(base_xc), (min(sp) if sp else None)

    head = [r for r in rows if r[1]["port"]["wall_median"] >= args.floor]
    small = [r for r in rows if r[1]["port"]["wall_median"] < args.floor]
    groups = [("**all (headline)**", head)]
    groups += [(e, [r for r in head if engine_of(r[0][0]) == e]) for e in ENGINES]
    fmts = sorted({r[0][1] for r in head})
    groups += [(f"-T{f}", [r for r in head if r[0][1] == f]) for f in fmts]
    groups += [(f"start-up (<{args.floor*1000:.0f} ms at 1.0)", small)]

    if new:
        print("| group | cases | ×1.0 | min ×1.0 | ×1.0 parallel | ×C now | ×C at 1.0 | RSS /1.0 |")
        print("| :-- | --: | --: | --: | --: | --: | --: | --: |")
    else:
        print("| group | cases | ×C at 1.0 |")
        print("| :-- | --: | --: |")
    for name, sel in groups:
        if not sel:
            continue
        cnt, sp, spc, xc, rss, bxc, mn = stats(sel)
        if new:
            print(f"| {name} | {cnt} | {fmt(sp)} | {fmt(mn)} | {fmt(spc)} | {fmt(xc)} | {fmt(bxc)} | {fmt(rss)} |")
        else:
            print(f"| {name} | {cnt} | {fmt(bxc)} |")

    if args.cases:
        print()
        if not new:
            print("| input | -T | 1.0 ms | C ms | ×C | RSS MiB | C RSS MiB |")
            print("| :-- | :-- | --: | --: | --: | --: | --: |")
            for (inp, f), b, _, _ in sorted(rows, key=lambda r: (ENGINES.index(engine_of(r[0][0])), r[0])):
                c = b.get("c") or {}
                ok = c.get("exit") == 0
                ct = c["wall_median"] if ok else None
                crss = c["rss_kib_max"] / 1024 if ok else None
                print(f"| {inp} | {f} | {b['port']['wall_median']*1000:,.1f} | {fmt(ct*1000 if ct else None, 1)} | "
                      f"{fmt(ct/b['port']['wall_median'] if ct else None)} | {b['port']['rss_kib_max']/1024:.1f} | "
                      f"{fmt(crss, 1)} |")
            return 0
        print("| input | -T | 1.0 ms | new ms | ×1.0 | C ms | ×C | RSS MiB 1.0 → new |")
        print("| :-- | :-- | --: | --: | --: | --: | --: | :-- |")
        for (inp, f), b, n, _ in sorted(rows, key=lambda r: (ENGINES.index(engine_of(r[0][0])), r[0])):
            bt = b["port"]["wall_median"]
            c = b.get("c") or {}
            ct = c.get("wall_median") if c.get("exit") == 0 else None
            nt = n["port"]["wall_median"] if n else None
            ref = nt or bt
            rss = f"{b['port']['rss_kib_max']/1024:.1f}"
            if n:
                rss += f" → {n['port']['rss_kib_max']/1024:.1f}"
            print(f"| {inp} | {f} | {bt*1000:,.1f} | {fmt(nt*1000 if nt else None, 1)} | "
                  f"{fmt(bt/nt if nt else None)} | {fmt(ct*1000 if ct else None, 1)} | "
                  f"{fmt(ct/ref if ct else None)} | {rss} |")
    return 0


if __name__ == "__main__":
    sys.exit(main())
