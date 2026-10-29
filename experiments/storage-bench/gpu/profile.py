#!/usr/bin/env python3
"""Separate counter pass, never substitute profiler timing for primary CUDA-event samples."""
import csv
import io
import json
import os
from pathlib import Path
import subprocess
import run

NCU = '/opt/nvidia/hpc_sdk/Linux_x86_64/25.9/profilers/13.0/Nsight_Compute/ncu'
OUT = run.RAW / 'counters'
OUT.mkdir(exist_ok=True)

def collect():
    for job in run.JOBS:
        stem = OUT / '-'.join(job)
        status = stem.with_suffix('.status.json')
        if status.exists() and json.loads(status.read_text())['returncode'] == 0:
            continue
        command = ['nice', '-n', '19', NCU, '--profile-from-start', 'off', '--cache-control', 'none', '--clock-control', 'none', '--replay-mode', 'kernel', '--metrics', 'dram__bytes_read.sum,dram__bytes_write.sum', '--csv', '--log-file', str(stem.with_suffix('.csv')), str(run.HERE / 'bench-profile'), *job]
        with stem.with_suffix('.jsonl').open('w') as stdout, stem.with_suffix('.stderr').open('w') as stderr:
            result = subprocess.run(command, env={**os.environ, 'BENCH_PROFILE': '1'}, stdout=stdout, stderr=stderr)
        status.write_text(json.dumps({'job': job, 'returncode': result.returncode, 'command': command, 'stderr': stem.with_suffix('.stderr').read_text()}) + '\n')
        print('counters', job, 'exit', result.returncode, flush=True)
        if result.returncode:
            break


def integrate():
    path = run.RESULTS / ('gpu-' + run.DATE + '.json')
    report = json.loads(path.read_text())
    profiles = []
    failures = []
    for job in run.JOBS:
        stem = OUT / '-'.join(job)
        status = stem.with_suffix('.status.json')
        if not status.exists():
            failures.append({'job': job, 'reason': 'counter pass not run'})
            continue
        st = json.loads(status.read_text())
        if st['returncode']:
            failures.append(st)
            continue
        samples = []
        for line in stem.with_suffix('.jsonl').read_text().splitlines():
            try:
                r = json.loads(line)
                if r.get('type') == 'sample' and r['round'] == 0:
                    samples.append(r)
            except json.JSONDecodeError:
                pass
        text = stem.with_suffix('.csv').read_text()
        lines = text.splitlines()
        start = next((i for i, line in enumerate(lines) if line.startswith('"ID"')), None)
        if start is None:
            failures.append({'job': job, 'reason': 'no NCU CSV header', 'log': text})
            continue
        rows = list(csv.DictReader(io.StringIO('\n'.join(lines[start:]))))
        kernels = {}
        for row in rows:
            if row.get('Metric Name') not in ('dram__bytes_read.sum', 'dram__bytes_write.sum'):
                continue
            kid = int(row['ID'])
            entry = kernels.setdefault(kid, {'kernel': row['Kernel Name'], 'id': kid})
            value = float(row['Metric Value'].replace(',', ''))
            unit = row['Metric Unit']
            factor = {'byte': 1, 'Kbyte': 1000, 'Mbyte': 1000000, 'Gbyte': 1000000000}.get(unit)
            if factor is None:
                raise ValueError((unit, row))
            entry[row['Metric Name']] = value * factor
        ordered = [kernels[k] for k in sorted(kernels)]
        pos = 0
        for sample in samples:
            count = 0 if sample['experiment'] == 'M4' and sample['variant'] == 'plain' else 1
            if sample['experiment'] == 'M3' and sample['variant'] == 'for':
                count = next((i + 1 for i, k in enumerate(ordered[pos:]) if 'encodek(' in k['kernel']), len(ordered) - pos + 1)
            used = ordered[pos:pos + count]
            pos += count
            if len(used) != count:
                failures.append({'job': job, 'reason': 'missing kernel records', 'sample': sample})
                continue
            profiles.append({**sample, 'kernels': used, 'dram_read_bytes': sum(k['dram__bytes_read.sum'] for k in used), 'dram_write_bytes': sum(k['dram__bytes_write.sum'] for k in used)})
        if pos != len(ordered):
            failures.append({'job': job, 'reason': 'extra kernel records', 'consumed': pos, 'observed': len(ordered)})
    for r in report['results']:
        found = [p for p in profiles if (p['experiment'], p['case'], p['variant']) == (r['experiment'], r['case'], r['variant'])]
        if len(found) != 1:
            continue
        p = found[0]
        r['profile_exact'] = p['exact']
        r['profile_valid'] = p['valid']
        r['profile_dram_read_bytes'] = p['dram_read_bytes']
        r['profile_dram_write_bytes'] = p['dram_write_bytes']
        r['profile_read_GBps_using_unprofiled_median'] = p['dram_read_bytes'] / r['median_ms'] / 1e6
        r['profile_read_write_floor_fraction'] = (p['dram_read_bytes'] + p['dram_write_bytes']) / r['median_ms'] / 1e6 / 960
        if r['experiment'] == 'M4':
            r['transferred_H2D_bytes'] = r['stored_bytes']
            r['effective_H2D_GBps'] = r['stored_bytes'] / r['median_ms'] / 1e6
            r['effective_PCIe_floor_fraction'] = r['effective_H2D_GBps'] / 31.508
    report['counter_protocol'] = 'Separate Nsight Compute pass, five warmups plus one profiled sample per variant; cudaProfilerStart/Stop brackets timed work only; kernel replay, cache-control none, clock-control none; dram__bytes_read.sum and dram__bytes_write.sum. Counter bytes are divided by the original 21-sample unprofiled median. No CI for the one-sample physical traffic count; cache/clock perturbation and replay can affect comparability. Memcpy engines are not represented in kernel DRAM counters; M4 reports exact submitted H2D bytes separately.'
    report['counter_interpretation'] = 'Zero/near-zero DRAM counts are real observations for cache-resident kernels, not missing measurements; a DRAM floor is not the active floor for those cases. Lazy cache writeback can fall outside the kernel counter interval. In the 15%-request small sweep, plain reads approximately zero DRAM bytes through 16 bits, 36.34 MB at 20 bits and 48.01 MB at 24/32 bits: observed traffic supports the cache-regime explanation for the reversed width crossover. At 60,012,150 values, plain count approaches 88% of the 960 GB/s DRAM floor, whereas 4-bit FOR reaches about 8% despite much lower byte traffic: fewer bytes do not imply a faster decoder. M2 is cache-resident in this protocol. The 960 GB/s floor is the advertised approximate peak; sampled memory clocks are recorded, not locked, and no clock-adjusted peak is claimed.'
    report['counter_profiles'] = profiles
    report['counter_failures'] = failures
    if profiles:
        report['metric_limitations'][0] = 'Original modeled metrics are retained for reproducibility. Additional profile_* fields contain observed DRAM traffic from a separate NCU run, not simultaneous observations on the primary 21 timed samples; profiler replay/cache perturbation limits comparability.'
    path.write_text(json.dumps(report, indent=2) + '\n')
    md = run.RESULTS / ('gpu-' + run.DATE + '.md')
    text = md.read_text()
    text = text.split('\n## Physical-traffic counter pass\n')[0]
    if profiles:
        text = text.replace('No hardware bandwidth counters were collected.', 'A separate hardware-counter pass is appended below.')
        text = text.replace('Hardware traffic profiling was not performed; actual DRAM GB/s and an actual attained-bandwidth-floor fraction are not established.', 'Separate hardware-counter measurements are appended below; original tables retain their explicitly modeled metrics.')
    text += '\n## Physical-traffic counter pass\n\n' + report['counter_protocol'] + '\n\n'
    text += report['counter_interpretation'] + '\n\n'
    text += '|Experiment / case|Variant|DRAM read MB|DRAM write MB|Read GB/s / unprofiled median|Read+write / 960 GB/s floor|\n|---|---|---:|---:|---:|---:|\n'
    for r in report['results']:
        if 'profile_dram_read_bytes' in r:
            text += f"|{r['experiment']} / {r['case']}|{r['variant']}|{r['profile_dram_read_bytes']/1e6:.3f}|{r['profile_dram_write_bytes']/1e6:.3f}|{r['profile_read_GBps_using_unprofiled_median']:.2f}|{r['profile_read_write_floor_fraction']:.1%}|\n"
    text += '\nCounter failures: ' + (json.dumps(failures) if failures else 'none') + '\n'
    text += '\n## Reproduction\n\nFrom the repository root:\n\n```sh\nnice -n 19 /opt/nvidia/hpc_sdk/Linux_x86_64/25.9/cuda/13.0/bin/nvcc -O3 -arch=sm_89 -std=c++17 experiments/storage-bench/gpu/bench.cu -o experiments/storage-bench/gpu/bench\nnice -n 19 python3 experiments/storage-bench/gpu/run.py --force\nnice -n 19 /opt/nvidia/hpc_sdk/Linux_x86_64/25.9/cuda/13.0/bin/nvcc -O3 -arch=sm_89 -std=c++17 experiments/storage-bench/gpu/bench.cu -o experiments/storage-bench/gpu/bench-profile\nnice -n 19 python3 experiments/storage-bench/gpu/profile.py\n```\n\nThe counter collector resumes successful existing profiles. Primary runs without `--force` resume successful groups; `--only merge --force` reruns only M3. `run.py --summarize` regenerates the primary summary; subsequently run `profile.py` to reattach existing counter records. All preprocessing is single-threaded and excluded from timing. No host CPU benchmarks were run.\n'
    md.write_text(text)
    print('integrated', len(profiles), 'counter profiles;', len(failures), 'counter failures', flush=True)

if __name__ == '__main__':
    collect()
    integrate()
