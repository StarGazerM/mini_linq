#!/usr/bin/env python3
"""P5 S1: five interleaved pairs, 21 samples after five warmups per process."""
import fcntl
import json
import os
from pathlib import Path
import random
import resource
import statistics
import subprocess

ROOT = Path(__file__).resolve().parents[2]
BINARY = ROOT / 'target/release/examples/scan_bench'
rng = random.Random(5101)
report = {'protocol': '5 interleaved pairs; 5 warmups + 21 samples/process; entire series flock', 'pairs': []}
cases = [(n,bits,op) for n in [4_194_304,67_108_864] for bits in [8,16] for op in ['filter','mask']]
cases += [(4_194_304,16,op) for op in ['encode','decode']]
with open('/tmp/mini_linq-timing.lock', 'w') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    report['load_start'] = os.getloadavg()
    for n,bits,operation in cases:
        for pair in range(5):
            runs = {}
            for variant in ['vec', 'bytes']:
                before = resource.getrusage(resource.RUSAGE_CHILDREN)
                result = subprocess.run([str(BINARY), variant, operation, str(n), str(bits)], cwd=ROOT, text=True, capture_output=True, check=True)
                after = resource.getrusage(resource.RUSAGE_CHILDREN)
                data = json.loads(result.stdout)
                data['minor_faults'] = after.ru_minflt-before.ru_minflt
                data['major_faults'] = after.ru_majflt-before.ru_majflt
                data['median_ns'] = statistics.median(data['samples_ns'])
                runs[variant] = data
            report['pairs'].append({'rows': n, 'bits': bits, 'operation': operation, 'pair': pair, 'runs': runs})
    report['load_end'] = os.getloadavg()
report['summary'] = {}
for n,bits,operation in cases:
    pairs = [p['runs'] for p in report['pairs'] if (p['rows'],p['bits'],p['operation']) == (n,bits,operation)]
    ratios = [p['bytes']['median_ns']/p['vec']['median_ns'] for p in pairs]
    boots = sorted(statistics.median(rng.choices(ratios,k=5)) for _ in range(10000))
    report['summary'][f'{n}/{bits}/{operation}'] = {
        'bytes_over_vec': statistics.median(ratios), 'bootstrap_95_ci': [boots[250],boots[9750]],
        'million_rows_per_second': {v: n*1000/statistics.median(p[v]['median_ns'] for p in pairs) for v in ['vec','bytes']},
        'input_GB_per_second': {v: n*(8 if v == 'vec' or operation == 'encode' else bits/8)/statistics.median(p[v]['median_ns'] for p in pairs) for v in ['vec','bytes']},
        'minor_faults': {v: sum(p[v]['minor_faults'] for p in pairs) for v in ['vec','bytes']},
        'major_faults': {v: sum(p[v]['major_faults'] for p in pairs) for v in ['vec','bytes']},
    }
output = ROOT/'experiments/storage-bench/results/p5-s1-scan-vectorized.json'
output.write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report['summary'],indent=2))
