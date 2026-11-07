#!/usr/bin/env python3
"""Rotated fresh-process comparison of GPU TPC-H base-column codecs (STORAGE E1).

One cq-gpu-tpch binary runs every variant (`--base-codec`), so code layout is
identical across variants. Each round runs every variant once per scale, in a
rotated order; round -1 is a discarded warmup. Each process holds the GPU lock
only for its own run. Every row must be validated under the tools/README
timing-check policy (warmup and first measured sample full exact comparisons,
later samples bench-check sampled); `exact` is reserved for all-samples-full.
"""
import argparse
from datetime import datetime, timezone
import fcntl
import json
import math
import os
from pathlib import Path
import random
import resource
import subprocess
import time

LOCK = '/tmp/mini_linq-gpu.lock'


def now():
    return datetime.now(timezone.utc).isoformat()


def foreign_tenants():
    out = subprocess.run(['nvidia-smi', '--query-compute-apps=pid,process_name,used_memory',
                          '--format=csv,noheader'], capture_output=True, text=True, timeout=5)
    return [line for line in out.stdout.splitlines() if line.strip()]


def gpu_state():
    out = subprocess.run(['nvidia-smi', '--query-gpu=clocks.sm,clocks.mem,temperature.gpu,'
                          'power.draw,utilization.gpu', '--format=csv,noheader'],
                         capture_output=True, text=True, timeout=5)
    return out.stdout.strip()


def variants(cache):
    return {
        'for': ['--base-codec', 'for'],
        'plain64': ['--base-codec', 'plain64'],
        'native': ['--base-codec', 'native'],
        'hostfor': ['--base-codec', 'hostfor'],
        # Warmup round writes the packed files; measured rounds must all hit.
        'hostfor-cached': ['--base-codec', 'hostfor', '--base-cache', str(cache)],
    }


def run_one(binary, output, scale, codec, queries, mode, cache):
    argv = [str(binary), str(output), '--scale', scale, '--mode', mode] + variants(cache)[codec]
    for query in queries:
        argv += ['--query', query]
    with open(LOCK, 'w') as lock:
        waited = time.monotonic()
        fcntl.flock(lock, fcntl.LOCK_EX)
        waited = time.monotonic() - waited
        tenants = foreign_tenants()
        record = dict(argv=argv, utc=now(), lock_wait_s=waited, tenants_before=tenants,
                      loadavg=os.getloadavg(), gpu_before=gpu_state())
        before = resource.getrusage(resource.RUSAGE_CHILDREN)
        began = time.monotonic()
        proc = subprocess.run(argv, cwd=binary.parent, capture_output=True, text=True)
        record['process_s'] = time.monotonic() - began
        after = resource.getrusage(resource.RUSAGE_CHILDREN)
    record.update(exit_code=proc.returncode, stderr_tail=proc.stderr[-4000:],
                  minflt=after.ru_minflt - before.ru_minflt,
                  majflt=after.ru_majflt - before.ru_majflt)
    record['per_query'] = {}
    try:
        report = json.loads(output.read_text())
        for row in report['results']:
            record['per_query'][row['query']] = dict(
                status=row.get('status'), error=row.get('error'), exact=row.get('exact'),
                validated=row.get('validated'), validation=row.get('validation'),
                median_ms=row.get('median_ms'), base_columns=row.get('base_columns'),
                total_ms=[s['total_ms'] for s in row.get('samples', [])],
                checks=[s.get('check') for s in row.get('samples', [])])
    except (OSError, ValueError, KeyError) as error:
        record['results_error'] = str(error)
    record['validated'] = (proc.returncode == 0 and bool(record['per_query']) and all(
        q['status'] == 'ran' and not q['error'] and q['validated'] is True
        and (q['validation'] or {}).get('full_samples', 0) >= 1
        for q in record['per_query'].values()))
    record['exact'] = record['validated'] and all(
        q['exact'] is True for q in record['per_query'].values())
    return record


def load_ms(columns):
    return columns['read_ms'] + columns.get('pack_ms', 0) + columns['upload_encode_ms']


def geomean(values):
    return math.exp(sum(map(math.log, values)) / len(values))


def bootstrap(ratios, seed, resamples=20000):
    rng = random.Random(seed)
    means = sorted(geomean([rng.choice(ratios) for _ in ratios]) for _ in range(resamples))
    return [means[int(0.025 * resamples)], means[int(0.975 * resamples) - 1]]


def metric(run, query, name):
    if name == 'process':
        return run['process_s']
    row = run['per_query'][query]
    if name == 'total':
        return row['median_ms']['total']
    columns = row['base_columns']
    if not columns or not columns['columns']:
        return None
    return load_ms(columns) if name == 'load' else columns['upload_encode_ms']


def analyze(runs, codecs, seed):
    """Per (scale, query, variant): geomean over rounds of the paired ratio to
    `for` of process-median query total_ms, base-column load ms (read + host
    pack + upload/device encode) and upload/encode ms alone. `suite` pairs the
    per-round geomean over queries; `process` pairs whole-process wall time."""
    table = {}
    measured = [r for r in runs if not r['warmup']]
    for scale in sorted({r['scale'] for r in measured}):
        rounds = sorted({r['round'] for r in measured if r['scale'] == scale})
        by = {(r['round'], r['codec']): r for r in measured if r['scale'] == scale}
        queries = sorted(by[(rounds[0], 'for')]['per_query'])
        for codec in codecs:
            if codec == 'for':
                continue
            out = table.setdefault(scale, {}).setdefault(codec, {})
            for name in ('total', 'load', 'upload_encode'):
                per_round = []
                for query in queries:
                    ratios = []
                    for rnd in rounds:
                        x, y = metric(by[(rnd, codec)], query, name), metric(by[(rnd, 'for')], query, name)
                        if x and y:
                            ratios.append(x / y)
                    if len(ratios) == len(rounds):
                        out.setdefault(query, {})[name] = dict(
                            ratio=geomean(ratios), ci95=bootstrap(ratios, seed), pairs=len(ratios))
                        per_round.append(ratios)
                if per_round:
                    suite = [geomean([q[i] for q in per_round]) for i in range(len(rounds))]
                    out.setdefault('suite', {})[name] = dict(
                        ratio=geomean(suite), ci95=bootstrap(suite, seed), pairs=len(suite),
                        queries=len(per_round))
            process = [by[(rnd, codec)]['process_s'] / by[(rnd, 'for')]['process_s'] for rnd in rounds]
            out.setdefault('suite', {})['process'] = dict(
                ratio=geomean(process), ci95=bootstrap(process, seed), pairs=len(process))
    return table


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--results', type=Path, required=True)
    parser.add_argument('--codecs', default='for,plain64,native,hostfor,hostfor-cached')
    parser.add_argument('--scales', default='sf1,sf10')
    parser.add_argument('--queries', default='', help='comma list; empty = all')
    parser.add_argument('--rounds', type=int, default=5)
    parser.add_argument('--mode', default='advised')
    parser.add_argument('--seed', type=int, default=20261106)
    args = parser.parse_args()
    codecs, scales = args.codecs.split(','), args.scales.split(',')
    queries = [q for q in args.queries.split(',') if q]
    binary = args.binary.resolve()
    work = Path('/dev/shm') / f'gpu-base-codec-{os.getpid()}'
    cache = work / 'base-cache'  # private to this invocation
    assert 'for' in codecs and args.rounds >= 1 and set(codecs) <= variants(cache).keys()
    work.mkdir()
    runs = []
    for rnd in range(-1, args.rounds):
        for scale in scales:
            order = [codecs[(max(rnd, 0) + i) % len(codecs)] for i in range(len(codecs))]
            for codec in order:
                output = work / f'r{rnd}-{scale}-{codec}.json'
                for attempt in range(3):
                    record = run_one(binary, output, scale, codec, queries, args.mode, cache)
                    if not record['tenants_before']:
                        break
                    time.sleep(60)  # contaminated sample: discard and retry
                else:
                    raise SystemExit('foreign GPU tenant persisted; no clean sample')
                record.update(round=rnd, warmup=rnd < 0, scale=scale, codec=codec, attempts=attempt + 1)
                if codec == 'hostfor-cached' and rnd >= 0 and any(
                        q['base_columns'] and q['base_columns']['cache_hits'] != q['base_columns']['columns']
                        for q in record['per_query'].values()):
                    record['validated'] = False
                    record['cache_error'] = 'measured hostfor-cached run missed the packed cache'
                runs.append(record)
                print(json.dumps(dict(round=rnd, scale=scale, codec=codec, validated=record['validated'],
                                      process_s=round(record['process_s'], 2))), flush=True)
                if not record['validated']:
                    args.results.parent.mkdir(parents=True, exist_ok=True)
                    args.results.write_text(json.dumps(dict(runs=runs), indent=1))
                    raise SystemExit(f'unvalidated or failed run: {output}')
    summary = analyze(runs, codecs, args.seed)
    args.results.parent.mkdir(parents=True, exist_ok=True)
    args.results.write_text(json.dumps(dict(
        protocol=dict(rounds=args.rounds, warmup_rounds=1, codecs=codecs, scales=scales,
                      queries=queries or 'all', mode=args.mode, seed=args.seed,
                      bootstrap_resamples=20000, lock=LOCK,
                      binary=str(binary),
                      binary_sha256=subprocess.run(['sha256sum', str(binary)],
                                                   capture_output=True, text=True).stdout.split()[0]),
        summary=summary, runs=runs), indent=1))


if __name__ == '__main__':
    main()
