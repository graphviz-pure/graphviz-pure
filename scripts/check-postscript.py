#!/usr/bin/env python3
"""Compare -Tps (or -Tps2, -Teps) bytes across the engine corpora (no comment normalization).

Set GVBINDIR and LD_PRELOAD for the C oracle; the port runs without either.
Run at nice 15, with --out on a scratch filesystem. Only mismatches are saved.
"""
import argparse
import concurrent.futures
import json
import os
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--oracle", required=True)
    parser.add_argument("--port", required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=180)
    parser.add_argument("--format", choices=["ps", "ps2", "eps"], default="ps")
    parser.add_argument("--engine", choices=["dot", "neato", "fdp", "sfdp", "circo", "twopi", "osage", "patchwork"])
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    port_env = dict(os.environ)
    for key in ["LD_PRELOAD", "GVBINDIR"]:
        port_env.pop(key, None)
    corpora = [("wild", "dot"), ("fixtures", "dot"), ("render", "dot")]
    corpora += [(e, e) for e in ["neato", "fdp", "sfdp", "circo", "twopi", "osage", "patchwork"]]
    jobs = [(e, f) for d, e in corpora if args.engine in (None, e)
            for f in sorted(Path("tests", d).glob("*.gv"))]

    def run(binary, argv, env):
        try:
            result = subprocess.run([binary, *argv], env=env, stdout=subprocess.PIPE,
                                    stderr=subprocess.PIPE, timeout=args.timeout)
            return result.returncode, result.stdout
        except subprocess.TimeoutExpired:
            return "timeout", b""

    def compare(item):
        engine, path = item
        argv = ["-K" + engine, "-T" + args.format, str(path)]
        c_status, expected = run(args.oracle, argv, os.environ)
        p_status, actual = run(args.port, argv, port_env)
        same = isinstance(c_status, int) and c_status >= 0 and c_status == p_status and expected == actual
        row = dict(engine=engine, file=str(path), c=c_status, port=p_status, same=same)
        if not same:
            tag = engine + "__" + path.stem
            (args.out / (tag + ".c." + args.format)).write_bytes(expected)
            (args.out / (tag + ".rs." + args.format)).write_bytes(actual)
            if c_status == 0:
                versions = {expected}
                for _ in range(4):
                    status, data = run(args.oracle, argv, os.environ)
                    versions.add(data)
                    if status != 0:
                        row["c_repeat_status"] = status
                row["c_distinct_outputs"] = len(versions)
        return row

    rows = []
    with (args.out / "results.jsonl").open("w") as log:
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            for row in pool.map(compare, jobs):
                rows.append(row)
                log.write(json.dumps(row) + "\n")
                log.flush()
    for engine in dict.fromkeys(e for _, e in corpora):
        group = [r for r in rows if r["engine"] == engine]
        if group:
            print(engine, sum(r["same"] for r in group), "/", len(group))


if __name__ == "__main__":
    main()
