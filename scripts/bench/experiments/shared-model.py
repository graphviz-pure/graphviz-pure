#!/usr/bin/env python3
"""Paired diagnostic measurements; apply shared-model.patch only to the candidate.

Run under nice -n 15 and ulimit -v 8000000. Builds must be fresh v3 musl
PGO builds with the same training list. Never use the diagnostic in production.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import random
import statistics
import subprocess
import time


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('base', 'candidate', 'control', 'reference', 'synth', 'out'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--rounds', type=int, default=30)
    args = parser.parse_args()
    binaries = {k: getattr(args, k).resolve() for k in
                ('base', 'candidate', 'control', 'reference')}
    hashes = {k: digest(v) for k, v in binaries.items()}
    assert hashes['base'] == hashes['control'], 'control must be an identical copy'
    assert hashes['base'] != hashes['candidate'], 'candidate must differ'
    cases = [(f'{e}_clustered_{n}.gv', fmt) for e in ('osage', 'patchwork')
             for n, fmt in ((5000, 'svg'), (20000, 'svg'), (20000, 'plain'))]
    cases += [('dot_dag_1000.gv', 'svg'), ('twopi_tree_5000.gv', 'svg'),
              ('neato_sparse_250.gv', 'svg')]
    result = {'hashes': hashes, 'rounds': args.rounds, 'seed': 610,
              'uptime_start': subprocess.check_output(['uptime'], text=True).strip(),
              'cases': []}
    rng = random.Random(610)
    for name, fmt in cases:
        path = args.synth.resolve() / name
        # Hash complete output outside the timing; stderr diagnostics must also agree.
        outputs = {}
        for key in ('base', 'candidate'):
            run = subprocess.run([str(binaries[key]), '-T' + fmt, str(path)],
                                 capture_output=True, check=True)
            outputs[key] = (hashlib.sha256(run.stdout).hexdigest(), run.stderr.decode())
        assert outputs['base'] == outputs['candidate'], (name, fmt, outputs)
        rows = []
        for round_index in range(args.rounds):
            order = list(binaries)
            rng.shuffle(order)
            row = {}
            for key in order:
                start = time.perf_counter()
                run = subprocess.run(['/usr/bin/time', '-f', '%U %S %M %R',
                                      str(binaries[key]), '-T' + fmt, str(path)],
                                     stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                                     text=True, check=True)
                wall = time.perf_counter() - start
                user, system, rss, faults = run.stderr.splitlines()[-1].split()
                row[key] = dict(wall=wall, user=float(user), system=float(system),
                                rss=int(rss), faults=int(faults))
            rows.append(row)
        summary = {}
        for key in binaries:
            summary[key] = {metric: statistics.median(r[key][metric] for r in rows)
                            for metric in ('wall', 'user', 'system', 'rss', 'faults')}
        for key in ('candidate', 'control'):
            logs = [math.log(r['base']['wall'] / r[key]['wall']) for r in rows]
            boot = sorted(math.exp(statistics.mean(rng.choices(logs, k=len(logs))))
                          for _ in range(2000))
            summary[key]['paired_speedup'] = math.exp(statistics.mean(logs))
            summary[key]['bootstrap_95'] = [boot[50], boot[1949]]
        result['cases'].append(dict(input=name, format=fmt, output_sha256=outputs['base'][0],
                                    summary=summary, rows=rows))
        result['uptime_end'] = subprocess.check_output(['uptime'], text=True).strip()
        args.out.write_text(json.dumps(result, indent=2) + '\n')
        print(name, fmt, json.dumps(summary), flush=True)


if __name__ == '__main__':
    main()
