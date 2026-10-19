#!/usr/bin/env python3
"""Profile top-N kernel names from analyze.py JSON; one launch per name by default.

NCU replay perturbs timing: use nsys for latency/concurrency and this report for
per-launch saturation. Increase --launch-count to sample more launches.
"""
import argparse
import csv
import io
import json
import os
from pathlib import Path
import re
import subprocess
import time

METRICS = [
    'sm__throughput.avg.pct_of_peak_sustained_elapsed',
    'dram__throughput.avg.pct_of_peak_sustained_elapsed',
    'sm__warps_active.avg.pct_of_peak_sustained_active',
    'smsp__thread_inst_executed_per_inst_executed.ratio',
    'launch__grid_size', 'launch__block_size', 'gpu__time_duration.sum',
]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('analysis', type=Path)
    p.add_argument('output', type=Path)
    p.add_argument('--top', type=int, default=5)
    p.add_argument('--launch-count', type=int, default=1)
    p.add_argument('--profile-from-start', choices=['on', 'off'], default='on')
    p.add_argument('--nvtx-range', help='push/pop range to include, e.g. fixed-point')
    p.add_argument('--range-instance', type=int, help='one-based NVTX range occurrence')
    p.add_argument('--resume', action='store_true', help='reuse already captured reports in output')
    p.add_argument('command', nargs=argparse.REMAINDER)
    a = p.parse_args()
    command = a.command[1:] if a.command[:1] == ['--'] else a.command
    if not command:
        p.error('provide -- PROGRAM [ARGS...]')
    ncu = os.environ.get('NCU', '/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/compilers/bin/ncu')
    a.output.mkdir(parents=True, exist_ok=True)
    results = []
    for index, kernel in enumerate(json.loads(a.analysis.read_text())['top_kernels'][:a.top]):
        while subprocess.check_output(['nvidia-smi', '--query-compute-apps=pid', '--format=csv,noheader'], text=True).strip():
            time.sleep(2)
        prefix = a.output / f'kernel-{index + 1}'
        cmd = [ncu, '--target-processes', 'all', '--kernel-name-base', 'demangled',
               '--kernel-name', 'regex:^' + re.escape(kernel['name']) + '$',
               '--rename-kernels', 'off',
               '--launch-skip', str(kernel.get('representative_launch_skip', 0)),
               '--launch-count', str(a.launch_count), '--profile-from-start', a.profile_from_start,
               '--metrics', ','.join(METRICS), '--force-overwrite', '--export', str(prefix),
               '--csv', '--page', 'raw']
        if a.nvtx_range:
            cmd += ['--nvtx', '--nvtx-include', a.nvtx_range + '/']
        if a.range_instance is not None:
            if not a.nvtx_range:
                p.error('--range-instance requires --nvtx-range')
            cmd += ['--range-filter', f'no::{a.range_instance}']
        cmd += command
        if not (a.resume and prefix.with_suffix('.ncu-rep').exists() and prefix.with_suffix('.csv').exists()):
            run = subprocess.run(cmd, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            prefix.with_suffix('.csv').write_text(run.stdout)
            if run.returncode:
                raise SystemExit(f'NCU failed ({run.returncode}); see {prefix}.csv')
        lines = prefix.with_suffix('.csv').read_text().splitlines()
        header = next((i for i, line in enumerate(lines) if line.startswith('"ID",')), None)
        if header is None:
            raise SystemExit(f'No NCU metrics captured; see {prefix}.csv')
        metrics = []
        reader = csv.DictReader(io.StringIO('\n'.join(lines[header:])))
        units = {}
        for row in reader:
            if 'Metric Name' in row:
                if row.get('Metric Name') in METRICS:
                    metrics.append(row)
            elif not row.get('ID'):
                units = row
            else:
                for name in METRICS:
                    if name in row:
                        metrics.append({'ID': row['ID'], 'Kernel Name': row['Kernel Name'],
                                        'Block Size': row['Block Size'], 'Grid Size': row['Grid Size'],
                                        'Metric Name': name, 'Metric Value': row[name],
                                        'Metric Unit': units.get(name, '')})
        if not metrics:
            raise SystemExit(f'Empty NCU metric report; see {prefix}.csv')
        results.append({'kernel': kernel['name'], 'nsys_total_ms': kernel['total_ms'], 'metrics': metrics})
        (a.output / 'metrics.json').write_text(json.dumps(results, indent=2) + '\n')
    print(a.output / 'metrics.json')


if __name__ == '__main__':
    main()
