#!/usr/bin/env python3
"""Paired musl churn diagnostics with an exec-isolated wait4/RSS helper.

Run serially under nice -n 15 and ulimit -v 8000000. Baseline and candidate
must be separately trained v3 musl PGO builds; control must copy baseline.
Experimental patches in this directory are never applied to shipped code.
"""
import json
import os
import subprocess
import sys
import time


def measure(command):
    # A direct wait4 in the accumulating driver inherits its growing RSS floor.
    # Exec this helper anew for each run, and check that its floor is below target.
    with open('/proc/self/statm', encoding='ascii') as statm:
        driver_rss = int(statm.read().split()[1])
    driver_rss *= os.sysconf('SC_PAGE_SIZE') // 1024
    with open(os.devnull, 'wb') as sink:
        start = time.perf_counter_ns()
        process = subprocess.Popen(command, stdout=sink, stderr=sink)
        _, status, usage = os.wait4(process.pid, 0)
        wall = (time.perf_counter_ns() - start) / 1e9
        process.returncode = os.waitstatus_to_exitcode(status)
    assert process.returncode == 0, process.returncode
    assert usage.ru_maxrss > driver_rss, ('driver RSS floor', usage.ru_maxrss, driver_rss)
    print(json.dumps(dict(wall=wall, user=usage.ru_utime, kernel=usage.ru_stime,
                          rss=usage.ru_maxrss, minflt=usage.ru_minflt,
                          majflt=usage.ru_majflt, involuntary=usage.ru_nivcsw,
                          driver_rss=driver_rss)))


def main():
    import argparse
    import hashlib
    import math
    from pathlib import Path
    import random
    import statistics

    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('base', 'candidate', 'control', 'reference', 'synth', 'out'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--rounds', type=int, default=40)
    args = parser.parse_args()
    assert args.rounds >= 4
    binaries = {key: getattr(args, name).resolve() for key, name in
                [('base', 'base'), ('prop', 'candidate'), ('ctrl', 'control'),
                 ('ref1', 'reference')]}
    hashes = {key: hashlib.sha256(path.read_bytes()).hexdigest()
              for key, path in binaries.items()}
    assert hashes['base'] == hashes['ctrl'] != hashes['prop'], hashes
    rng = random.Random(6101134)
    result = dict(rounds=args.rounds, seed=6101134, hashes=hashes, cases=[],
                  uptime_start=subprocess.check_output(['uptime'], text=True).strip())
    cases = [(e, n, f) for e in ('osage', 'patchwork')
             for n, f in [(5000, 'svg'), (20000, 'svg'), (20000, 'plain')]]
    cases += [('dot', 1000, 'svg'), ('twopi', 20000, 'svg')]

    def run(key, command):
        helper = [sys.executable, str(Path(__file__).resolve()), '--measure',
                  str(binaries[key]), *command]
        return json.loads(subprocess.check_output(helper, text=True))

    for engine, nodes, fmt in cases:
        family = 'dag' if engine == 'dot' else 'tree' if engine == 'twopi' else 'clustered'
        name = f'{engine}_{family}_{nodes}.gv'
        command = ['-K' + engine, '-T' + fmt, str(args.synth.resolve() / name)]
        outputs = {}
        for key in ('base', 'prop', 'ctrl'):
            process = subprocess.run([str(binaries[key]), *command],
                                     capture_output=True, check=True)
            outputs[key] = (hashlib.sha256(process.stdout).hexdigest(),
                            process.stderr.decode())
        assert len(set(outputs.values())) == 1, (name, fmt, outputs)
        for _ in range(3):
            order = list(binaries)
            rng.shuffle(order)
            for key in order:
                run(key, command)
        rows = []
        for index in range(args.rounds):
            order = list(binaries)
            rng.shuffle(order)
            rows.append(dict(round=index, order=order,
                             runs={key: run(key, command) for key in order}))
        metrics = ('wall', 'user', 'kernel', 'rss', 'minflt', 'majflt', 'involuntary')
        summary = {key: {m: statistics.median(row['runs'][key][m] for row in rows)
                         for m in metrics} for key in binaries}
        ratios = {}
        for numerator, denominator in [('base', 'prop'), ('base', 'ctrl')]:
            logs = [math.log(row['runs'][numerator]['wall'] /
                             row['runs'][denominator]['wall']) for row in rows]
            boot = sorted(math.exp(statistics.mean(rng.choices(logs, k=len(logs))))
                          for _ in range(5000))
            ratios[numerator + '/' + denominator] = dict(
                geomean=math.exp(statistics.mean(logs)), bootstrap95=[boot[125], boot[4874]])
        result['cases'].append(dict(engine=engine, n=nodes, format=fmt, input=name,
                                    output_sha256=outputs['base'][0], summary=summary,
                                    ratios=ratios, rows=rows))
        result['uptime_end'] = subprocess.check_output(['uptime'], text=True).strip()
        args.out.write_text(json.dumps(result, indent=1) + '\n')
        print(engine, nodes, fmt, json.dumps(summary), json.dumps(ratios), flush=True)


if __name__ == '__main__':
    if len(sys.argv) > 1 and sys.argv[1] == '--measure':
        measure(sys.argv[2:])
    else:
        main()
