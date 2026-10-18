#!/usr/bin/env python3
"""Assemble emitted PTX and attach measured ptxas resources to benchmark JSON."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument('--results', default='compare/gpu_h05.results.json')
parser.add_argument('--ptx-dir', default='target/oxide-ir')
args = parser.parse_args()
cuda = Path(os.environ.get('CUDA_HOME', '/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/cuda/12.9'))
reports = []
resources = {}
for ptx in sorted(Path(args.ptx_dir).glob('*.ptx')):
    if ptx.stem not in {'smoke', 'tiles_oracle', 'gpu_blocked_run', 'gpu_h05', 'blocked_run_oracle', 'dedup_oracle'}:
        continue
    proc = subprocess.run([str(cuda / 'bin/ptxas'), '-v', '-arch=sm_89', str(ptx), '-o', '/dev/null'], text=True, capture_output=True, check=True)
    report = proc.stdout + proc.stderr
    reports.append(f'FILE {ptx}\n{report}')
    current = None
    frame = stores = loads = 0
    for line in report.splitlines():
        match = re.search(r"Compiling entry function '([^']+)'", line)
        if match:
            current = match[1]
            frame = stores = loads = 0
        match = re.search(r'(\d+) bytes stack frame, (\d+) bytes spill stores, (\d+) bytes spill loads', line)
        if match:
            frame, stores, loads = map(int, match.groups())
        match = re.search(r'Used (\d+) registers', line)
        if match and current:
            smem = re.search(r'(\d+) bytes smem', line)
            resources[current] = {'regs': int(match[1]), 'stack_bytes': frame, 'spill_stores_bytes': stores, 'spill_loads_bytes': loads, 'static_smem_bytes': int(smem[1]) if smem else 0}
Path('compare').mkdir(exist_ok=True)
Path('compare/ptxas.txt').write_text('\n'.join(reports))
Path('compare/ptxas.resources.json').write_text(json.dumps(resources, indent=2) + '\n')
result_path = Path(args.results)
if result_path.exists():
    results = json.loads(result_path.read_text())
    for row in results:
        target = 'plan_' + row['plan'].lower()
        matches = [v for k, v in resources.items() if target in k]
        if len(matches) != 1:
            raise RuntimeError(f'expected one measured {target}, found {len(matches)}')
        value = matches[0]
        row['regs'] = value['regs']
        row['spills'] = {'stores_bytes': value['spill_stores_bytes'], 'loads_bytes': value['spill_loads_bytes']}
        row['stack_bytes'] = value['stack_bytes']
        row['static_smem_bytes'] = value['static_smem_bytes']
    result_path.write_text(json.dumps(results, indent=2) + '\n')
print('\n'.join(reports))
