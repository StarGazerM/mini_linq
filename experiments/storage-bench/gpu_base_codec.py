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
        # FastLanes DELTA where detect_integer_codec picks it, FOR elsewhere.
        'delta': ['--base-codec', 'delta'],
        # Warmup round writes the packed files; measured rounds must all hit.
        'hostfor-cached': ['--base-codec', 'hostfor', '--base-cache', str(cache)],
        # Read control: raw columns from tmpfs (--tmpfs-manifests), like the packed files.
        'for-tmpfs': ['--base-codec', 'for'],
    }


def manifest(args, scale, codec):
    """TPCH_GPU_MANIFEST for one process: every variant reads the same
    verified-key manifest; `for-tmpfs` reads its tmpfs copy of the raw columns."""
    if codec == 'for-tmpfs':
        return args.tmpfs_manifests / f'{scale}-tmpfs.json'
    return args.manifests / f'{scale}.json' if args.manifests else None


def run_one(binary, output, scale, codec, queries, mode, cache, manifest_path):
    argv = [str(binary), str(output), '--scale', scale, '--mode', mode] + variants(cache)[codec]
    for query in queries:
        argv += ['--query', query]
    env = dict(os.environ)
    if manifest_path:
        env['TPCH_GPU_MANIFEST'] = str(manifest_path)
    with open(LOCK, 'w') as lock:
        waited = time.monotonic()
        fcntl.flock(lock, fcntl.LOCK_EX)
        waited = time.monotonic() - waited
        tenants = foreign_tenants()
        record = dict(argv=argv, manifest=str(manifest_path) if manifest_path else None,
                      utc=now(), lock_wait_s=waited, tenants_before=tenants,
                      loadavg=os.getloadavg(), gpu_before=gpu_state())
        before = resource.getrusage(resource.RUSAGE_CHILDREN)
        began = time.monotonic()
        proc = subprocess.run(argv, cwd=binary.parent, env=env, capture_output=True, text=True)
        record['process_s'] = time.monotonic() - began
        after = resource.getrusage(resource.RUSAGE_CHILDREN)
        record['tenants_after'] = foreign_tenants()
        record['loadavg_after'] = os.getloadavg()
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
                checks=[s.get('check') for s in row.get('samples', [])],
                authored_comparison=row.get('authored_comparison'))
        record['data_root'] = report.get('data_root')
    except (OSError, ValueError, KeyError) as error:
        record['results_error'] = str(error)
    answers = bool(record['per_query']) and all(
        q['status'] == 'ran' and not q['error'] and q['validated'] is True
        and (q['validation'] or {}).get('full_samples', 0) >= 1
        for q in record['per_query'].values())
    # The binary also fails its exit status when advised H05 SF10 exceeds the
    # default build's 10.35 ms ceiling: a timing outcome of the variant, recorded
    # separately, not an answer failure.
    record['h05_ceiling_failed'] = any(
        (q['authored_comparison'] or {}).get('within_sf10_ceiling') is False
        for q in record['per_query'].values())
    record['validated'] = answers and (proc.returncode == 0 or record['h05_ceiling_failed'])
    record['exact'] = record['validated'] and all(
        q['exact'] is True for q in record['per_query'].values())
    return record


def load_ms(columns):
    return sum(columns.get(key, 0) for key in
               ('read_ms', 'verify_ms', 'pack_ms', 'upload_encode_ms', 'catalog_ms'))


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
    `for` of process-median query total_ms, base-column load ms (read + set
    verification + host pack + upload/device encode) and upload/encode ms
    alone. `suite` pairs the per-round geomean over queries; `process` pairs
    whole-process wall time."""
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


def filesystem(path):
    """Filesystem type of the canonical file (statfs), not a path-prefix guess."""
    out = subprocess.run(['stat', '-f', '-c', '%T', os.path.realpath(path)],
                         capture_output=True, text=True, check=True)
    return out.stdout.strip()


def check_tmpfs_control(args, scales):
    """`for-tmpfs` must differ from `for` only in where raw columns live: same
    manifest (verified keys included) up to column/dictionary paths, the same
    bytes, and every tmpfs file on tmpfs. Relative base paths resolve against
    `--data-root`; every run must report that root (checked after each run)."""
    if not args.manifests or not args.tmpfs_manifests:
        raise SystemExit('for-tmpfs requires --manifests and --tmpfs-manifests')
    evidence = {}
    for scale in scales:
        base = json.loads((args.manifests / f'{scale}.json').read_text())
        control = json.loads((args.tmpfs_manifests / f'{scale}-tmpfs.json').read_text())
        root = args.data_root / scale
        pairs = []
        for table, relation in base['relations'].items():
            for column, moved in zip(relation['columns'], control['relations'][table]['columns']):
                for key in ('file', 'dictionary'):
                    if column.get(key):
                        pairs.append((root / column[key], Path(moved[key])))
                        moved[key] = column[key]
        if control != base:
            raise SystemExit(f'{scale}: tmpfs manifest differs beyond column paths')
        for original, moved in pairs:
            if not moved.is_absolute() or filesystem(moved) != 'tmpfs':
                raise SystemExit(f'{moved} is not an absolute tmpfs file')
            if subprocess.run(['cmp', '-s', str(original), str(moved)]).returncode:
                raise SystemExit(f'{moved} differs from {original}')
        evidence[scale] = dict(files=len(pairs), identical=True, tmpfs=True, data_root=str(root))
    return evidence


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--results', type=Path, required=True)
    parser.add_argument('--codecs', default='for,plain64,native,hostfor,hostfor-cached,for-tmpfs')
    parser.add_argument('--manifests', type=Path,
                        help='directory of sfN.json manifests (TPCH_GPU_MANIFEST) for every variant')
    parser.add_argument('--tmpfs-manifests', type=Path,
                        help='directory of sfN-tmpfs.json manifests whose columns live in tmpfs')
    parser.add_argument('--data-root', type=Path, default=Path('/data/tpch-gpu'),
                        help="the binary's compiled TPC-H root; each run must report it")
    parser.add_argument('--scales', default='sf1,sf10')
    parser.add_argument('--queries', default='', help='comma list; empty = all')
    parser.add_argument('--rounds', type=int, default=5)
    parser.add_argument('--mode', default='advised')
    parser.add_argument('--seed', type=int, default=20261106)
    parser.add_argument('--max-load', type=float, default=8.0,
                        help='1-minute load average above which a sample is discarded')
    parser.add_argument('--max-wait', type=float, default=12 * 3600,
                        help='seconds to wait for a clean GPU/host before giving up')
    args = parser.parse_args()
    codecs, scales = args.codecs.split(','), args.scales.split(',')
    queries = [q for q in args.queries.split(',') if q]
    binary = args.binary.resolve()
    args.manifests = args.manifests.resolve() if args.manifests else None
    args.tmpfs_manifests = args.tmpfs_manifests.resolve() if args.tmpfs_manifests else None
    args.data_root = args.data_root.resolve()
    work = Path('/dev/shm') / f'gpu-base-codec-{os.getpid()}'
    cache = work / 'base-cache'  # private to this invocation
    assert 'for' in codecs and args.rounds >= 1 and set(codecs) <= variants(cache).keys()
    controls = check_tmpfs_control(args, scales) if 'for-tmpfs' in codecs else None
    work.mkdir()
    runs, discarded = [], []

    def clean():
        return not foreign_tenants() and os.getloadavg()[0] <= args.max_load
    for rnd in range(-1, args.rounds):
        for scale in scales:
            order = [codecs[(max(rnd, 0) + i) % len(codecs)] for i in range(len(codecs))]
            for codec in order:
                output = work / f'r{rnd}-{scale}-{codec}.json'
                attempt, deadline = 0, time.monotonic() + args.max_wait
                while True:
                    while not clean():
                        if time.monotonic() > deadline:
                            raise SystemExit('no clean GPU/host within --max-wait')
                        time.sleep(60)
                    if time.monotonic() > deadline:
                        raise SystemExit('no clean sample within --max-wait')
                    attempt += 1
                    record = run_one(binary, output, scale, codec, queries, args.mode, cache,
                                     manifest(args, scale, codec))
                    # Foreign GPU tenants or host load before/after a sample contaminate it.
                    if (not record['tenants_before'] and not record['tenants_after']
                            and max(record['loadavg'][0], record['loadavg_after'][0]) <= args.max_load):
                        break
                    discarded.append(dict(round=rnd, scale=scale, codec=codec, utc=record['utc'],
                                          tenants_before=record['tenants_before'],
                                          tenants_after=record['tenants_after'],
                                          loadavg=record['loadavg'], loadavg_after=record['loadavg_after']))
                    args.results.parent.mkdir(parents=True, exist_ok=True)
                    args.results.write_text(json.dumps(dict(partial=True, discarded=discarded, runs=runs)))
                record.update(round=rnd, warmup=rnd < 0, scale=scale, codec=codec, attempts=attempt)
                if record.get('data_root') != str(args.data_root):
                    record['validated'] = False
                    record['root_error'] = f"binary read {record.get('data_root')}, not --data-root"
                if codec == 'hostfor-cached' and rnd >= 0 and any(
                        q['base_columns']
                        and q['base_columns']['cache_hits'] != q['base_columns']['unprojected']
                        for q in record['per_query'].values()):
                    record['validated'] = False
                    record['cache_error'] = 'measured hostfor-cached run missed the packed cache'
                runs.append(record)
                print(json.dumps(dict(round=rnd, scale=scale, codec=codec, validated=record['validated'],
                                      process_s=round(record['process_s'], 2))), flush=True)
                # Partial evidence survives an interrupted campaign.
                args.results.parent.mkdir(parents=True, exist_ok=True)
                args.results.write_text(json.dumps(dict(partial=True, discarded=discarded, runs=runs)))
                if not record['validated']:
                    raise SystemExit(f'unvalidated or failed run: {output}')
    summary = analyze(runs, codecs, args.seed)
    args.results.parent.mkdir(parents=True, exist_ok=True)
    args.results.write_text(json.dumps(dict(
        protocol=dict(rounds=args.rounds, warmup_rounds=1, codecs=codecs, scales=scales,
                      queries=queries or 'all', mode=args.mode, seed=args.seed,
                      manifests=str(args.manifests) if args.manifests else None,
                      tmpfs_manifests=str(args.tmpfs_manifests) if args.tmpfs_manifests else None,
                      tmpfs_control=controls, data_root=str(args.data_root),
                      max_load=args.max_load, discard_rule='foreign GPU tenant or 1-min load above max_load, before or after the sample',
                      bootstrap_resamples=20000, lock=LOCK,
                      binary=str(binary),
                      binary_sha256=subprocess.run(['sha256sum', str(binary)],
                                                   capture_output=True, text=True).stdout.split()[0]),
        summary=summary, discarded=discarded, runs=runs), indent=1))


if __name__ == '__main__':
    main()
