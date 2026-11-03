#!/usr/bin/env python3
"""Bounded, rotated fresh-process TPC-H comparisons across N >= 2 bundles."""
import argparse
from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import resource
import signal
import subprocess
import sys
import time


def now():
    return datetime.now(timezone.utc).isoformat()


def rotation_order(count, round_index):
    return [(round_index + offset) % count for offset in range(count)]


def schedule(count, scales, pairs):
    return [dict(round=r, warmup=r < 0, scale=scale, bundle=index)
            for r in range(-1, pairs) for scale in scales
            for index in rotation_order(count, max(r, 0))]


def gpu_state():
    state = {'utc': now()}
    for name, query in (
            ('gpu', '--query-gpu=index,uuid,name,clocks.sm,clocks.mem,temperature.gpu,power.draw,utilization.gpu'),
            ('compute_processes', '--query-compute-apps=gpu_uuid,pid,process_name,used_memory')):
        try:
            result = subprocess.run(['nvidia-smi', query, '--format=csv,noheader'],
                                    capture_output=True, text=True, timeout=2)
            state[name] = dict(stdout=result.stdout, stderr=result.stderr, exit_code=result.returncode)
        except (OSError, subprocess.TimeoutExpired) as error:
            state[name] = {'error': str(error)}
    return state


def load_transfer_fields(value):
    """Retain load/transfer accounting even when nested under inputs or timing."""
    if isinstance(value, dict):
        result = {}
        for key, child in value.items():
            if 'load' in key.lower() or 'transfer' in key.lower():
                result[key] = child
            else:
                nested = load_transfer_fields(child)
                if nested:
                    result[key] = nested
        return result
    if isinstance(value, list):
        return {str(index): fields for index, child in enumerate(value)
                if (fields := load_transfer_fields(child))}
    return {}


def parse_results(report, queries):
    per_query = {}
    for entry in report['results']:
        if entry['query'] not in queries:
            continue
        if entry['query'] in per_query:
            raise ValueError('multiple results for requested query ' + entry['query'])
        per_query[entry['query']] = {
            key: entry.get(key) for key in ('exact', 'answer_rows', 'median_ms', 'samples')}
        per_query[entry['query']]['load_transfer'] = load_transfer_fields(entry)
    missing = set(queries) - per_query.keys()
    if missing:
        raise ValueError('missing requested queries: ' + ', '.join(sorted(missing)))
    return per_query


class Bundle:
    def __init__(self, directory):
        self.dir = directory
        self.manifest = json.loads((directory / 'manifest.json').read_text())
        self.worktree = Path(self.manifest['worktree'])
        self.env = os.environ.copy()
        self.env.update(LD_LIBRARY_PATH=f'{directory / "lib"}:/usr/local/nvidia/lib64:/usr/local/nvidia/lib',
                        BUNDLE_DIR=str(directory), DUCKDB_CLI=str(directory / 'bin/duckdb'),
                        PATH=str(directory / 'bin') + ':' + self.env.get('PATH', '/usr/bin:/bin'))
        self.env.pop('CUDA_HOME', None)
        self.env.pop('CUDA_OXIDE_PTX_DIR', None)

    def metadata(self, index):
        binary = self.manifest['binaries']['cq-gpu-tpch']
        return dict(index=index, commit=self.manifest['commit'], dir=str(self.dir),
                    worktree=str(self.worktree), builds=self.manifest.get('builds', []),
                    binary_sha256=self.manifest.get('files', {}).get(binary))


def run_process(bundle, output, queries, scale, mode, timeout):
    argv = [str(bundle.dir / bundle.manifest['binaries']['cq-gpu-tpch']), str(output)]
    argv += [arg for query in queries for arg in ('--query', query)]
    argv += ['--scale', scale, '--mode', mode]
    record = dict(argv=argv, utc=now(), results_json=str(output), per_query={}, timed_out=False)
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    began = time.monotonic()
    try:
        proc = subprocess.Popen(argv, cwd=bundle.worktree / 'experiments/gpu', env=bundle.env,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                                start_new_session=True)
        try:
            stdout, stderr = proc.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            stdout, stderr = proc.communicate()
            record['timed_out'] = True
        record.update(exit_code=proc.returncode, stdout=stdout, stderr=stderr)
    except OSError as error:
        record.update(exit_code=None, error=str(error))
    record['process_seconds'] = time.monotonic() - began
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    record['rusage_delta'] = {name: getattr(after, 'ru_' + name) - getattr(before, 'ru_' + name)
                              for name in ('minflt', 'majflt', 'maxrss')}
    # Linux CHILDREN maxrss is a high-water mark, not a per-process RSS measurement.
    record['children_maxrss_kib'] = dict(before=before.ru_maxrss, after=after.ru_maxrss)
    try:
        report = json.loads(output.read_text())
        record['per_query'] = parse_results(report, queries)
        record['load_transfer'] = load_transfer_fields(report)
    except (OSError, ValueError, KeyError, TypeError) as error:
        record['results_error'] = str(error)
    record['exact'] = (set(record['per_query']) == set(queries)
                       and all(row['exact'] is True for row in record['per_query'].values()))
    record['failed'] = record['exit_code'] != 0 or not record['exact']
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--results', type=Path, required=True)
    parser.add_argument('--queries', default='h06')
    parser.add_argument('--scales', default='sf1,sf10')
    parser.add_argument('--pairs', type=int, default=7)
    parser.add_argument('--budget', type=float, required=True, help='wall seconds including warmup')
    parser.add_argument('--mode', default='advised', choices=('advised', 'automatic'))
    args = parser.parse_args()
    queries, scales = args.queries.split(','), args.scales.split(',')
    if (args.pairs < 1 or not math.isfinite(args.budget) or args.budget <= 0
            or not all(queries) or len(set(queries)) != len(queries)
            or not scales or any(scale not in ('sf1', 'sf10') for scale in scales)
            or len(set(scales)) != len(scales)):
        parser.error('require positive pairs/budget, unique queries, and unique sf1/sf10 scales')
    if 'BUNDLE_DIRS' in os.environ:
        dirs = [Path(p).resolve() for p in json.loads(os.environ['BUNDLE_DIRS'])]
    else:
        current = Path(os.environ.get('BUNDLE_DIR', __file__)).resolve()
        root = next(p for p in (current, *current.parents) if p.name.startswith('bundle-')).parent
        dirs = sorted((p.resolve() for p in root.glob('bundle-*') if (p / 'manifest.json').is_file()),
                      key=lambda p: int(p.name.removeprefix('bundle-')))
    if len(dirs) < 2:
        parser.error('require at least two bundles')
    current = Path(os.environ.get('BUNDLE_DIR', __file__)).resolve()
    if not current.is_relative_to(dirs[-1]):
        print('pod-bench: deferred to the last bundle invocation', flush=True)
        return 0
    # Reuse the runner's ownership checks; same-worktree variants must switch the
    # mirror before EVERY process, not just when constructing bundle objects.
    sys.path.insert(0, str(dirs[-1] / 'tools'))
    from pod_gate import mirror
    deadline = time.monotonic() + args.budget
    bundles = [Bundle(directory) for directory in dirs]
    mirror('/data/tpch-gpu', '/workspace/data/tpch-gpu')
    args.results = args.results.resolve()
    args.results.mkdir(parents=True, exist_ok=True)
    path = args.results / 'pod-bench.json'
    data = dict(version=1, started_utc=now(), bundles=[b.metadata(i) for i, b in enumerate(bundles)],
                queries=queries, scales=scales, mode=args.mode, pairs=args.pairs,
                budget_seconds=args.budget, rounds=[], processes=[],
                not_run=schedule(len(bundles), scales, args.pairs), stop_reason=None,
                rusage_note='maxrss delta is the change in Linux cumulative child high-water RSS (KiB), not process peak RSS')

    def save():
        temporary = path.with_suffix('.tmp')
        temporary.write_text(json.dumps(data, indent=2) + '\n')
        temporary.replace(path)

    save()
    estimates = {}
    stopped = False
    for round_index in range(-1, args.pairs):
        state = dict(round=round_index, warmup=round_index < 0, before=gpu_state())
        data['rounds'].append(state)
        while data['not_run'] and data['not_run'][0]['round'] == round_index:
            item = data['not_run'][0]
            remaining = deadline - time.monotonic() - 4  # reserve the after-round GPU snapshot
            estimate = estimates.get((item['bundle'], item['scale']),
                                     max((v for (b, s), v in estimates.items() if s == item['scale']), default=0))
            if remaining <= 0 or estimate > remaining:
                data['stop_reason'] = 'budget: insufficient time for next process'
                stopped = True
                break
            bundle = bundles[item['bundle']]
            mirror(bundle.worktree, bundle.dir / 'source')
            output = args.results / f'round-{round_index}-{item["scale"]}-bundle-{item["bundle"]}.json'
            record = run_process(bundle, output, queries, item['scale'], args.mode, remaining)
            record.update(item)
            data['processes'].append(record)
            data['not_run'].pop(0)
            estimates[item['bundle'], item['scale']] = max(estimate, record['process_seconds'])
            save()
            if record['timed_out']:
                data['stop_reason'] = 'budget: process exceeded remaining time'
                stopped = True
                break
        state['after'] = gpu_state()
        save()
        if stopped:
            break
    data['finished_utc'] = now()
    save()
    return int(any(record['failed'] for record in data['processes']))


if __name__ == '__main__':
    sys.exit(main())
