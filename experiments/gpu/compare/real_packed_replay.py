#!/usr/bin/env python3
"""Local-only clean E/P/C real-read replay, with same-process ratios and five fresh processes."""
import argparse
import collections
import fcntl
import hashlib
import json
from pathlib import Path
import re
import statistics
import subprocess


def tenants():
    return subprocess.check_output(['nvidia-smi', '--query-compute-apps=pid,process_name', '--format=csv,noheader'], text=True).strip()


def distribution(values):
    return dict(median=statistics.median(values), minimum=min(values), maximum=max(values), n=len(values))


def add(target, row):
    target['groups'] += 1
    for field in ('probes', 'enumerated_rows'):
        target[field] += row[field]
    for field in ('ns', 'gpu_ns'):
        for mode in range(3):
            target[field][mode] += row[field][mode] / row.get('repeats', 1)


def empty():
    return dict(groups=0, probes=0, enumerated_rows=0, ns=[0, 0, 0], gpu_ns=[0, 0, 0])


def reduce_sample(report):
    axes = {name: collections.defaultdict(empty) for name in ('total', 'kind_stratified', 'order', 'site_launch_size', 'rule_launch_size', 'state_size', 'storage', 'site')}
    launch_probes = collections.Counter()
    for row in report['sites']:
        launch_probes[row['launch']] += row['probes']
    counts = collections.Counter()
    opportunities = collections.Counter()
    identities = {}
    for row in report['sites']:
        p = row['probes']
        size = row['state_bytes']
        def launch_class(n):
            return '<32' if n < 32 else '32-1023' if n < 1024 else '1024-29999' if n < 30000 else '>=30000'
        storage = 'unchanged_legacy_or_scalar' if row['site'].get('nonradix') or 'whole_source' in row['site'] else 'all_plain' if not row['packed_probes'] else 'all_packed' if row['packed_probes'] == p else 'mixed_runs'
        state_size = '<=128KiB' if size <= 128*1024 else '128KiB-1MiB' if size <= 2**20 else '1MiB-8MiB' if size <= 8*2**20 else '>8MiB'
        identity = f"{row['kernel']}/{row['site']['site']}"
        identities[identity] = dict(rule=row['rule'], site=row['site'], relation=row.get('relation'), kernel=row['kernel'])
        for axis, key in [('total', 'all'), ('order', row['order']), ('site_launch_size', launch_class(p)),
                          ('rule_launch_size', launch_class(launch_probes[row['launch']])), ('state_size', state_size), ('storage', storage), ('site', identity)]:
            add(axes[axis][key], row)
        for kind, diagnostic in row['stratified_kind_diagnostics'].items():
            add(axes['kind_stratified'][kind], dict(diagnostic, repeats=row['repeats']))
        counts.update(row['counts'])
        for key in ('run_domain_negative_opportunity', 'block_domain_negative_opportunity', 'packed_probes', 'packed_misses', 'empty_run_misses'):
            opportunities[key] += row.get(key, 0)
    admission = {}
    for mode, prefix in [('engine', 'plain'), ('plain', 'packed'), ('packed', 'packed')]:
        entry = report['admission'][mode]
        admission[mode] = dict(ns=sum(s[f'{prefix}_admission_ns'] for s in entry['states']),
                               pool_peak=entry['pool_peak'], pool_live=entry['pool_live'],
                               state_bytes=sum(s[f'{prefix}_bytes'] for s in entry['states']))
        if entry['timing_indicative']:
            raise RuntimeError('contended admission measurements')
    return dict(axes=axes, counts=counts, opportunities=opportunities, admission=admission, identities=identities,
                model=report['model'], launch_count=len(launch_probes))


def summarize(paths):
    samples = [reduce_sample(json.loads(path.read_text())) for path in paths]
    first = samples[0]
    summary = dict(samples=len(samples), model=first['model'], axes={}, counts=first['counts'], opportunities=first['opportunities'], admission={}, projection={})
    for sample in samples[1:]:
        if sample['counts'] != first['counts'] or sample['opportunities'] != first['opportunities']:
            raise ValueError('fresh processes did not replay identical records')
    heaviest = sorted(first['axes']['site'], key=lambda site: statistics.median(s['axes']['site'][site]['gpu_ns'][0] for s in samples), reverse=True)[:10]
    for axis in first['axes']:
        summary['axes'][axis] = {}
        labels = heaviest if axis == 'site' else first['axes'][axis]
        for label in labels:
            rows = [s['axes'][axis][label] for s in samples]
            out = {key: rows[0][key] for key in ('groups', 'probes', 'enumerated_rows')}
            for key in ('ns', 'gpu_ns'):
                out[key] = [distribution([r[key][mode] for r in rows]) for mode in range(3)]
                out[key + '_ratios'] = {name: distribution([r[key][a] / r[key][b] for r in rows]) for name, a, b in [('P/E', 1, 0), ('C/E', 2, 0), ('C/P', 2, 1)]}
            if axis == 'site':
                out['identity'] = first['identities'][label]
            summary['axes'][axis][label] = out
    for mode in ('engine', 'plain', 'packed'):
        summary['admission'][mode] = {field: distribution([s['admission'][mode][field] for s in samples]) for field in first['admission'][mode]}
    for mode, index in [('engine', 0), ('plain', 1), ('packed', 2)]:
        walls = []
        for sample in samples:
            reads = sample['axes']['total']['all']['gpu_ns']
            admissions = sample['admission']
            walls.append(333 + 262 * reads[index]/reads[0] + 74 * admissions[mode]['ns']/admissions['engine']['ns'])
        summary['projection'][mode] = distribution(walls)
    summary['projection']['basis'] = '[INFERENCE] Supplied prior wall partition: reads 262ms, admission 74ms, no-kernel 333ms. Replay GPU interval ratios and same-process admission wall ratios; not an engine measurement.'
    return summary


def measure(args):
    if args.processes < 5:
        raise ValueError('at least five fresh processes are required')
    args.output.mkdir(parents=True, exist_ok=True)
    paths = []
    for process in range(args.processes):
        path = args.output / f'process-{process}.json'
        if path.exists():
            raise ValueError(f'refusing to overwrite an existing measurement: {path}')
        with open('/tmp/mini_linq-gpu.lock', 'w') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            before = tenants()
            (args.output / f'process-{process}.before').write_text(before)
            if before:
                raise RuntimeError(f'GPU not quiet: {before}')
            try:
                with (args.output / f'process-{process}.log').open('w') as log:
                    subprocess.run([str(args.binary.resolve()), str(args.admission.resolve()), '--real-trace', str(args.trace.resolve()), '--output', str(path.resolve())], stdout=log, stderr=subprocess.STDOUT, check=True)
            finally:
                after = tenants()
                (args.output / f'process-{process}.after').write_text(after)
            if after:
                raise RuntimeError(f'GPU not quiet after run: {after}')
        paths.append(path)
        print(f'completed clean process {process}', flush=True)
    summary = summarize(paths)
    (args.output / 'summary.json').write_text(json.dumps(summary, indent=2))
    (args.output / 'binary.sha256').write_text(hashlib.sha256(args.binary.read_bytes()).hexdigest() + '\n')
    print(json.dumps(summary['axes']['total']['all']), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    p = sub.add_parser('measure')
    for name in ('binary', 'admission', 'trace', 'output'):
        p.add_argument(name, type=Path)
    p.add_argument('--processes', type=int, default=5)
    p = sub.add_parser('summarize'); p.add_argument('output', type=Path); p.add_argument('samples', nargs='+', type=Path)
    args = parser.parse_args()
    if args.command == 'measure':
        measure(args)
    else:
        args.output.write_text(json.dumps(summarize(args.samples), indent=2))
