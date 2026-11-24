#!/usr/bin/env python3
"""Single-host-thread orchestration; benchmark subprocesses monitor every CUDA event sample."""
import datetime
import json
import math
import os
from pathlib import Path
import random
import statistics
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
RESULTS = HERE.parent / 'results'
RESULTS.mkdir(parents=True, exist_ok=True)
DATE = '2026-10-28'
RAW = RESULTS / ('gpu-' + DATE + '-raw')
RAW.mkdir(exist_ok=True)
SIZES = (6001215, 60012150)
WIDTHS = (4, 8, 12, 16, 20, 24, 32)
JOBS = [('q6', str(t)) for t in (1, 10)]
JOBS += [('sweep', str(n), str(w)) for n in SIZES for w in WIDTHS]
JOBS += [('group', str(p)) for p in (0, 1)]
JOBS += [('merge', str(n)) for n in (1000000, 16000000)]
JOBS += [('transfer', str(n), str(w)) for n in SIZES for w in WIDTHS]
JOBS += [('transfer', str(n), '24', 'real') for n in SIZES]

def percentile(xs, q):
    s = sorted(xs)
    x = (len(s) - 1) * q
    lo = int(x)
    return s[lo] + (s[min(lo + 1, len(s) - 1)] - s[lo]) * (x - lo)

def summarize():
    records, failures = [], []
    for job in JOBS:
        path = RAW / ('-'.join(job) + '.jsonl')
        status = path.with_suffix('.status.json')
        if status.exists():
            info = json.loads(status.read_text())
            if info['returncode']:
                failures.append(info)
        elif path.exists():
            failures.append({'job': job, 'reason': 'incomplete subprocess'})
        else:
            failures.append({'job': job, 'reason': 'not run'})
        if path.exists():
            for line in path.read_text().splitlines():
                try:
                    records.append(json.loads(line))
                except json.JSONDecodeError:
                    failures.append({'job': job, 'reason': 'truncated JSON record'})
    groups = {}
    samples = {}
    for r in records:
        if r['type'] == 'group':
            groups[(r['experiment'], r['case'])] = r
        elif r['type'] == 'sample' and r['valid'] and r['round'] >= 0:
            samples.setdefault((r['experiment'], r['case'], r['variant']), []).append(r)
    rng = random.Random(20261028)
    results = []
    for key, values in sorted(samples.items()):
        exp, case, variant = key
        baseline = 'plain_offset' + variant.rsplit('offset', 1)[1] if exp == 'M2' else 'plain'
        plain = samples.get((exp, case, baseline), [])
        rounds = {r['round']: r['ms'] for r in plain}
        paired = [(r['ms'], rounds[r['round']]) for r in values if r['round'] in rounds]
        ts = [r['ms'] for r in values]
        median = statistics.median(ts)
        boot, ratios = [], []
        for _ in range(5000):
            draw = [paired[rng.randrange(len(paired))] for _ in paired]
            a = statistics.median(x[0] for x in draw)
            b = statistics.median(x[1] for x in draw)
            boot.append(a)
            ratios.append(a / b)
        ci = [percentile(boot, .025), percentile(boot, .975)]
        r = values[0]
        floor_gbps = 31.508 if exp == 'M4' else 960.0
        metric_bytes = r['logical_read_bytes']
        result = {
            'experiment': exp, 'case': case, 'variant': variant,
            'n': r['n'], 'samples': len(values), 'warmups': sum(x.get('type') == 'sample' and x.get('case') == case and x.get('variant') == variant and x.get('round', 0) < 0 and x.get('valid', False) for x in records),
            'median_ms': median, 'median_ms_bootstrap_95ci': ci,
            'ratio_to_plain': median / statistics.median(x['ms'] for x in plain),
            'ratio_bootstrap_95ci': [percentile(ratios, .025), percentile(ratios, .975)],
            'baseline': baseline, 'ns_per_value': median * 1e6 / r['n'],
            'stored_bytes': r['stored_bytes'],
            'modeled_access_bytes': metric_bytes,
            'modeled_effective_GBps': metric_bytes / median / 1e6,
            'bandwidth_floor_GBps': floor_gbps,
            'modeled_floor_fraction': metric_bytes / median / 1e6 / floor_gbps,
            'exact': all(x['exact'] for x in values),
            'probe_M_per_s': r['n'] / median / 1000 if exp == 'M2' else None,
            'sample_ms': ts,
        }
        results.append(result)
    report = {
        'date': DATE, 'target': 'RTX 6000 Ada sm_89',
        'protocol': {'warmups': 5, 'samples': 21, 'interleaving': 'rotating variant order within each round', 'timing': 'CUDA events, default stream; scalar output reset included for M1', 'bootstrap': '5000 paired-round resamples, percentile 95% CI of median and ratio of medians', 'exactness': 'Every warmup and measured sample: M1 exact scalar CPU reference; M2 every probe checked against CPU reference; M3/M4 every output element checked on GPU against uploaded CPU reference, mismatch flag copied to host', 'host_threads': 1, 'build': 'nice -n 19 nvcc -O3 -arch=sm_89 -std=c++17', 'tenant_policy': 'nvidia-smi before and after every timed sample; foreign-tenant samples invalidated and retried; wait up to ten minutes for each occupied sample'},
        'layout': {'block_values': 1024, 'metadata': 'u64 base + u64 byte offset + u8 width per block, separate arrays', 'for': '32-lane interleaved bit packing: lane l stores values l,l+32,... as one bit stream; 32-bit words transposed across lanes', 'bytes': 'per-block u8/u16/u32 offsets from minimum', 'dictionary': 'global sorted distinct u64 values, densely assigned codes, packed or byte-aligned; dictionary decoding for all operations, no code-domain shortcut', 'merge_output': 'Compact FOR output: a metadata kernel computes base/width and byte sizes, CUB exclusive scan builds compact offsets, and one 32-thread CTA per block packs output; all three stages are timed. Output capacity and CUB scratch are preallocated; output validation decodes all values'},
        'metric_limitations': ['Effective GB/s and floor fractions are modeled storage/access-byte metrics, not measured DRAM transactions. No hardware bandwidth counters were collected. Cache-line overfetch, repeated cached metadata loads, dictionary cache reuse and co-rank searches are not physically measured.', 'M1 count modeled bytes = entire encoded footprint including dictionary. Q6 model weights each column footprint by its actual short-circuit visit fraction, not cache-line-rounded traffic.', 'M2 reports probes/s, not a bandwidth floor: binary search and variable expansion cannot be reduced to the full stored footprint per probe.', 'M3 model counts plain input+output; FOR model counts encoded input plus decoded input write/read and merged output write/read plus compact output bytes, but excludes repeated merge co-rank reads and metadata scan traffic. Therefore its fraction is a lower-bound traffic model, not attained physical bandwidth.', 'M4 floor = theoretical PCIe 4.0 x16 payload encoding limit 31.508 GB/s; end-to-end bytes variant includes all four metadata/data transfers plus decode and output writes. Host encoding excluded.'],
        'effort': 'One generic decoder shared by all kernels. Same block/grid/reduction/merge primitive for every variant. No variant-specific launch tuning, no parameter search, no code-domain dictionary optimization. Plain/bytes/FOR/dict use uniform runtime layout dispatch.',
        'environment': [r for r in records if r['type'] == 'environment'],
        'groups': list(groups.values()), 'results': results, 'failures': failures,
        'tenant_records': [r for r in records if r['type'] == 'tenant_wait'],
        'raw_samples': [r for r in records if r['type'] == 'sample'],
    }
    conclusions = []
    for experiment in ('M1_q6', 'M2', 'M3', 'M4'):
        rows = [r for r in results if r['experiment'] == experiment]
        wins = {}
        for case in sorted({r['case'] for r in rows}):
            winner = min((r for r in rows if r['case'] == case), key=lambda r: r['median_ms'])
            wins[winner['variant']] = wins.get(winner['variant'], 0) + 1
        if rows:
            conclusions.append(f"{experiment}: lowest-median variant counts across cases: {wins}.")
    for n in SIZES:
        for variant in ('bytes', 'for', 'dict', 'dict_bytes'):
            selected = [r for r in results if r['experiment'] == 'M1_count' and r['variant'] == variant and r['n'] == n and r['case'].endswith('_s15')]
            wins = sorted(int(r['case'].split('_')[0][1:]) for r in selected if r['ratio_bootstrap_95ci'][1] < 1)
            losses = sorted(int(r['case'].split('_')[0][1:]) for r in selected if r['ratio_bootstrap_95ci'][0] > 1)
            conclusions.append(f"M1 count, n={n}, representative 15% request: {variant} significantly beats plain at widths {wins}, loses at {losses}; see all selectivities below.")
    conclusions.append("[INFERENCE] The small-input width crossover can reverse the usual expectation: increasing width also increases the aggregate cache working set of all five interleaved resident layouts. Thus a narrower case can favor cache-resident plain while a wider case favors compression; this is not a universal width-only cost rule. Dictionary gathers become particularly expensive as dictionary cardinality and footprint grow. Counter data can establish traffic, but not by itself identify the instruction/stall cause.")
    conclusions.append("M2 miss probes are uniformly sampled above the maximum key (hits are uniform over distinct keys), not uniform over absent interior gaps. Its directory is identical across codecs, but this miss distribution is a limitation of extrapolating absolute throughput.")
    report['conclusions'] = conclusions
    (RESULTS / ('gpu-' + DATE + '.json')).write_text(json.dumps(report, indent=2) + '\n')
    lines = ['# GPU storage-compression measurements — ' + DATE, '', 'RTX 6000 Ada; CUDA 13.0.48, `-O3 -arch=sm_89 -std=c++17`; one host thread, nice 19. These are isolated microbenchmarks, not end-to-end query claims.', '', '## Protocol', '', 'Five warmups and 21 measured samples per variant, round-interleaved with rotating first variant; CUDA events. Every warmup and sample checked exactly. 5,000 paired-round bootstrap resamples yield 95% percentile CIs. `nvidia-smi` tenant records surround **every** timed sample; clocks and machine load are recorded per group. JSON contains all raw timings, tenant snapshots, environment and clock records.', '', '## Results', '', 'Ratio is elapsed time / matched plain elapsed time; lower wins. M2 uses the same offset-width plain baseline. GB/s and floor fractions are **modeled**, not measured DRAM bandwidth (see limitations). GPU floor: 960 GB/s; M4 PCIe floor: 31.508 GB/s.', '', '|Experiment / case|Variant|Median ms [95% CI]|Ratio [95% CI]|ns/value|Modeled GB/s|Floor fraction|Exact / samples|', '|---|---|---:|---:|---:|---:|---:|---|']
    lines[4:4] = ['## Short analysis', ''] + ['- ' + s for s in conclusions] + ['']
    for r in results:
        ci, ri = r['median_ms_bootstrap_95ci'], r['ratio_bootstrap_95ci']
        bw = 'n/a' if r['experiment'] == 'M2' else f"{r['modeled_effective_GBps']:.2f}"
        frac = 'n/a' if r['experiment'] == 'M2' else f"{r['modeled_floor_fraction']:.1%}"
        lines.append(f"|{r['experiment']} / {r['case']}|{r['variant']}|{r['median_ms']:.5f} [{ci[0]:.5f}, {ci[1]:.5f}]|{r['ratio_to_plain']:.3f} [{ri[0]:.3f}, {ri[1]:.3f}]|{r['ns_per_value']:.4f}|{bw}|{frac}|{r['exact']} / {r['samples']}|")
    lines += ['', '## Per-experiment interpretation', '']
    for key in sorted(groups):
        rr = [r for r in results if (r['experiment'], r['case']) == key]
        if not rr:
            continue
        winner = min(rr, key=lambda r: r['median_ms'])
        detail = f"; {winner['probe_M_per_s']:.3f} M probes/s" if key[0] == 'M2' else ''
        lines.append(f"- **{key[0]} {key[1]}**: lowest observed median **{winner['variant']}**, ratio {winner['ratio_to_plain']:.3f}{detail}. This is a median ranking, not an automatic significance claim.")
    lines += ['', '### Width crossovers', '', 'Widths below are sampled discrete points, not interpolated thresholds. A win requires ratio CI upper bound <1; a loss requires lower bound >1. At 4 bits the requested 1% predicate rounds down to threshold 0 (actual selectivity 0%), because 1% is unrepresentable with a strict integer threshold on only 16 values. JSON records actual counts and thresholds.']
    for n in SIZES:
        for pct in (1, 15, 50):
            for variant in ('bytes', 'for', 'dict', 'dict_bytes'):
                wins, losses, overlaps = [], [], []
                for w in WIDTHS:
                    found = [r for r in results if r['case'] == f'w{w}_n{n}_s{pct}' and r['variant'] == variant]
                    if found:
                        lo, hi = found[0]['ratio_bootstrap_95ci']
                        (wins if hi < 1 else losses if lo > 1 else overlaps).append(w)
                lines.append(f'- n={n}, selectivity request={pct}%, {variant}: wins at {wins}; loses at {losses}; CI overlaps plain at {overlaps}.')
    lines += ['', '## Layout, fairness and interpretation constraints', '', report['layout']['for'] + '.', report['layout']['merge_output'] + '.', report['effort'], '', 'A codec can save bytes and still lose: this implementation adds per-row metadata/address/decode instructions; dictionaries also add dependent gathers. These are mechanistic explanations, not measured stall breakdowns. Small resident inputs can benefit from Ada’s cache, so footprint-based rates need not correspond to DRAM bandwidth. Q6 short-circuit access reduces the benefit of compressing late columns. M3 explicitly charges decode and re-encode; M4 explicitly charges compressed transfer plus decode, but not host compression.', '']
    lines += ['- ' + s for s in report['metric_limitations']]
    lines += ['', '## Incomplete work / failures', '']
    lines += ['- ' + json.dumps(f) for f in failures] or ['None: all listed benchmark groups completed. Hardware traffic profiling was not performed; actual DRAM GB/s and an actual attained-bandwidth-floor fraction are not established.']
    (RESULTS / ('gpu-' + DATE + '.md')).write_text('\n'.join(lines) + '\n')
    return report

if __name__ == '__main__':
    if '--summarize' not in sys.argv:
        for job in JOBS:
            if '--only' in sys.argv and job[0] != sys.argv[sys.argv.index('--only') + 1]:
                continue
            path = RAW / ('-'.join(job) + '.jsonl')
            status = path.with_suffix('.status.json')
            if '--force' not in sys.argv and status.exists() and json.loads(status.read_text())['returncode'] == 0:
                continue
            started = datetime.datetime.now(datetime.timezone.utc).isoformat()
            with path.open('w') as out, path.with_suffix('.stderr').open('w') as err:
                result = subprocess.run(['nice', '-n', '19', str(HERE / os.environ.get('BENCH_BINARY', 'bench')), *job], stdout=out, stderr=err)
            status.write_text(json.dumps({'job': job, 'returncode': result.returncode, 'started_utc': started, 'ended_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'stderr': path.with_suffix('.stderr').read_text()}) + '\n')
            print(job, 'exit', result.returncode, flush=True)
            if result.returncode:
                break
    report = summarize()
    print('summarized', len(report['results']), 'variants;', len(report['failures']), 'incomplete jobs', flush=True)
