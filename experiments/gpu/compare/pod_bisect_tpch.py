#!/usr/bin/env python3
"""Interleave historical TPC-H binaries; only the last bundle invocation runs."""
import argparse
import json
import math
import os
from pathlib import Path
import statistics
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--results', type=Path, required=True)
    parser.add_argument('--budget', type=float, default=2040)
    parser.add_argument('--rounds', type=int, default=3)
    parser.add_argument('--queries', nargs='+', default=['h09', 'h21_positive', 'h10', 'h03'])
    args = parser.parse_args()
    directories = [Path(p) for p in json.loads(os.environ['BUNDLE_DIRS'])]
    if Path(os.environ['BUNDLE_DIR']).resolve() != directories[-1].resolve():
        print('bisect: deferred to last bundle', flush=True)
        return 0
    start = time.monotonic()
    deadline = start + args.budget
    args.results.mkdir(parents=True, exist_ok=True)
    data = dict(rounds=args.rounds, budget_seconds=args.budget, scale='sf1', mode='advised',
                order='query-major within round; bundle order rotates by round + query index',
                bundles=[], samples=[], summary={}, complete=False)
    path = args.results / 'bisect-tpch.json'

    def save():
        data['elapsed_seconds'] = time.monotonic() - start
        temporary = path.with_suffix('.tmp')
        temporary.write_text(json.dumps(data, indent=2) + '\n')
        temporary.replace(path)

    try:
        for directory in directories:
            manifest = json.loads((directory / 'manifest.json').read_text())
            worktree = Path(manifest['worktree'])
            source = directory / 'source'
            if not worktree.exists() and not worktree.is_symlink():
                worktree.parent.mkdir(parents=True, exist_ok=True)
                worktree.symlink_to(source)
            if worktree.resolve() != source.resolve():
                raise RuntimeError(f'worktree mirror collision: {worktree}')
            data['bundles'].append(dict(commit=manifest['commit'], directory=str(directory),
                worktree=str(worktree), binary=str(directory / manifest['binaries']['cq-gpu-tpch'])))
        data['nvidia_smi'] = subprocess.check_output(['nvidia-smi'], text=True, timeout=10)
        save()
        for round_index in range(args.rounds):
            for query_index, query in enumerate(args.queries):
                shift = (round_index + query_index) % len(data['bundles'])
                order = data['bundles'][shift:] + data['bundles'][:shift]
                for bundle in order:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        raise TimeoutError('pod benchmark budget exhausted')
                    tenants = subprocess.check_output(['nvidia-smi', '--query-compute-apps=pid,process_name',
                        '--format=csv,noheader'], text=True, timeout=min(10, remaining)).strip()
                    if tenants:
                        raise RuntimeError('foreign GPU tenant: ' + tenants)
                    label = bundle['commit'][:8]
                    output = args.results / f'{round_index}-{query}-{label}.json'
                    log = args.results / f'{round_index}-{query}-{label}.log'
                    env = os.environ.copy()
                    env['LD_LIBRARY_PATH'] = f'{bundle["directory"]}/lib:/usr/local/nvidia/lib64:/usr/local/nvidia/lib'
                    env['DUCKDB_CLI'] = f'{bundle["directory"]}/bin/duckdb'
                    env['PATH'] = f'{bundle["directory"]}/bin:' + env.get('PATH', '/usr/bin:/bin')
                    env.pop('CUDA_HOME', None)
                    argv = [bundle['binary'], str(output), '--scale', 'sf1', '--mode', 'advised', '--query', query]
                    began = time.monotonic()
                    with log.open('w') as stream:
                        result = subprocess.run(argv, cwd=Path(bundle['worktree']) / 'experiments/gpu',
                            env=env, stdout=stream, stderr=subprocess.STDOUT,
                            timeout=max(0.001, deadline - time.monotonic()))
                    record = dict(commit=bundle['commit'], query=query, round=round_index,
                        process_seconds=time.monotonic() - began, exit_code=result.returncode,
                        argv=argv, report=str(output), log=str(log))
                    data['samples'].append(record)
                    save()
                    if result.returncode not in (0, 1):
                        raise RuntimeError(f'{label} {query}: exit {result.returncode}; see {log}')
                    report = json.loads(output.read_text())
                    rows = report['results']
                    if len(rows) != 1 or rows[0].get('query') != query or rows[0].get('mode') != 'advised' or rows[0].get('sf') != 1:
                        raise RuntimeError(f'{label} {query}: query selection mismatch')
                    row = rows[0]
                    if row.get('exact') is not True or row.get('status') != 'ran':
                        raise RuntimeError(f'{label} {query}: not exact: {row}')
                    timings = row['median_ms']
                    if not all(isinstance(timings.get(k), (float, int)) and math.isfinite(timings[k]) for k in ('build', 'kernels', 'total')):
                        raise RuntimeError(f'{label} {query}: invalid phase timings')
                    record.update(exact=True, median_ms=timings, samples=row['samples'],
                        answer_rows=row['answer_rows'], timing_boundary=row.get('timing_boundary'),
                        download_ms=timings.get('download'))
                    peers = [r for r in data['samples'] if r['commit'] == bundle['commit'] and r['query'] == query and r.get('exact')]
                    data['summary'].setdefault(label, {})[query] = dict(rounds=len(peers), exact=True,
                        median_ms={k: statistics.median(r['median_ms'][k] for r in peers) for k in ('build', 'kernels', 'dedup', 'total')})
                    print(f'{round_index} {label} {query}: {timings} ({record["process_seconds"]:.2f}s)', flush=True)
                    save()
        # Only the endpoint comparison has matching measurement boundaries.
        # The historical binaries lack separate downloads; never invent corrections.
        data['full_sf1'] = []
        endpoints = [data['bundles'][0], data['bundles'][-1]]
        for round_index in range(args.rounds):
            if deadline - time.monotonic() < 180:
                data['full_sf1_skipped'] = 'insufficient remaining budget for another full pair'
                break
            for bundle in (endpoints if round_index % 2 == 0 else endpoints[::-1]):
                label = bundle['commit'][:8]
                output = args.results / f'full-{round_index}-{label}.json'
                log = output.with_suffix('.log')
                env = os.environ.copy()
                env['LD_LIBRARY_PATH'] = f'{bundle["directory"]}/lib:/usr/local/nvidia/lib64:/usr/local/nvidia/lib'
                env['DUCKDB_CLI'] = f'{bundle["directory"]}/bin/duckdb'
                env['PATH'] = f'{bundle["directory"]}/bin:' + env.get('PATH', '/usr/bin:/bin')
                env.pop('CUDA_HOME', None)
                tenants = subprocess.check_output(['nvidia-smi', '--query-compute-apps=pid,process_name',
                    '--format=csv,noheader'], text=True, timeout=10).strip()
                if tenants:
                    raise RuntimeError('foreign GPU tenant: ' + tenants)
                began = time.monotonic()
                with log.open('w') as stream:
                    result = subprocess.run([bundle['binary'], str(output), '--scale', 'sf1', '--mode', 'advised'],
                        cwd=Path(bundle['worktree']) / 'experiments/gpu', env=env,
                        stdout=stream, stderr=subprocess.STDOUT,
                        timeout=max(0.001, deadline - time.monotonic()))
                if result.returncode not in (0, 1):
                    raise RuntimeError(f'{label} full suite exit {result.returncode}')
                report = json.loads(output.read_text())
                if len(report['results']) != 26 or any(r.get('exact') is not True for r in report['results']):
                    raise RuntimeError(f'{label} full suite did not return 26 exact queries')
                data['full_sf1'].append(dict(commit=bundle['commit'], round=round_index,
                    process_seconds=time.monotonic()-began, report=str(output), results=report['results']))
                save()
        data['complete'] = True
        save()
        return 0
    except Exception as error:
        data['error'] = f'{type(error).__name__}: {error}'
        save()
        raise


if __name__ == '__main__':
    raise SystemExit(main())
