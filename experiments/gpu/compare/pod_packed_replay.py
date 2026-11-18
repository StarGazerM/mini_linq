#!/usr/bin/env python3
"""Tenant-free, exact packed-run replay from a packaged source snapshot.

BUNDLE_DIR names a bundle containing source/, lib/, and manifest.json.
Each configuration gets its own fresh replay process. The replay itself always
runs both the engine and experimental paths; this driver selects the appropriate
column, so the three reported configurations each have exactly --reps samples.
"""
import argparse
import fcntl
import json
import math
import os
from pathlib import Path
import statistics
import subprocess
import sys
import time


TRACE = Path('/dev/shm/packed-run/trace')
CONFIGURATIONS = (
    ('engine-plain', (), 'plain', 'packed'),
    ('new-plain', ('--plain-storage',), 'packed', 'plain'),
    ('new-packed', (), 'packed', 'packed'),
)
CLASSES = ((0, 8), (9, 256), (257, 1024), (1025, 4096), (4097, 65536), (65537, None))


def tenants(env):
    return subprocess.run(
        ['nvidia-smi', '--query-compute-apps=pid,process_name', '--format=csv,noheader'],
        env=env, capture_output=True, text=True, check=True, timeout=10,
    ).stdout.strip()


def distribution(values):
    if not values or not all(isinstance(v, (int, float)) and math.isfinite(v) and v >= 0 for v in values):
        raise ValueError('missing or invalid measurement values')
    return dict(median=statistics.median(values), minimum=min(values), maximum=max(values), n=len(values))


def measurements(report, prefix):
    totals = report['totals']
    if prefix == 'plain':
        fresh = totals['plain_fresh_ns']
        seal = totals['plain_adopt_ns']
        merge = totals['plain_merge_publication_ns']
    else:
        fresh = totals['packed_sort_ns'] + totals['packed_subtract_ns']
        seal = totals['packed_seal_ns']
        merge = totals['packed_merge_ns']
    metrics = dict(admission_ms=totals[f'{prefix}_admission_ns'] / 1e6,
                   sort_subtract_ms=fresh / 1e6, seal_adopt_ms=seal / 1e6,
                   merge_ms=merge / 1e6, lookup_ms=totals[f'{prefix}_lookup_ns'] / 1e6,
                   final_pool_bytes=totals[f'{prefix}_pool_live'],
                   peak_pool_bytes=totals[f'{prefix}_pool_peak'])
    reads = {}
    for name, row in totals[f'{prefix}_reads'].items():
        engine = totals['plain_reads'][name]['ns']
        reads[name] = dict(milliseconds=row['ns'] / 1e6,
            engine_ratio=row['ns'] / engine if engine else 0,
            lookups_per_second=row['lookups_per_second'], rows_per_second=row['rows_per_second'],
            probes=row['probes'], rows_scanned=row['rows_scanned'])
    largest = {}
    for state in report['largest_states']:
        largest[str(state['id'])] = {
            name: dict(milliseconds=row['ns'] / 1e6,
                engine_ratio=row['ns'] / state['plain_reads'][name]['ns'] if state['plain_reads'][name]['ns'] else 0,
                lookups_per_second=row['lookups_per_second'], rows_per_second=row['rows_per_second'])
            for name, row in state[f'{prefix}_reads'].items() if name in reads}
    admissions = [a for state in report['states'] for a in state[f'{prefix}_admissions']]
    classes = {}
    for basis in ('candidate_rows', 'fresh_rows'):
        classes[basis] = {}
        for low, high in CLASSES:
            label = f'{low}-{high}' if high is not None else '>65536'
            selected = [a for a in admissions if a[basis] >= low and (high is None or a[basis] <= high)]
            values = {key: sum(a[key] for a in selected) / 1e6
                      for key in ('fresh_ns', 'subtract_ns', 'seal_ns', 'merge_ns')}
            classes[basis][label] = dict(admissions=len(selected),
                sort_or_fresh_ms=values['fresh_ns'], subtract_ms=values['subtract_ns'],
                seal_adopt_ms=values['seal_ns'], merge_ms=values['merge_ns'],
                admission_ms=sum(values.values()))
    return dict(metrics=metrics, classes=classes, reads=reads, largest_states=largest)


def summarize(samples):
    result = {}
    for block in sorted({s['block_rows'] for s in samples}):
        result[str(block)] = {}
        for name, _, _, _ in CONFIGURATIONS:
            selected = [s['measurements'] for s in samples if s['block_rows'] == block and s['configuration'] == name]
            if not selected:
                raise ValueError(f'missing samples for B={block} {name}')
            metrics = {key: distribution([s['metrics'][key] for s in selected]) for key in selected[0]['metrics']}
            classes = {}
            for basis, buckets in selected[0]['classes'].items():
                classes[basis] = {}
                for label, row in buckets.items():
                    classes[basis][label] = {
                        key: distribution([s['classes'][basis][label][key] for s in selected]) for key in row
                    }
            reads = {name: {key: distribution([s['reads'][name][key] for s in selected]) for key in row}
                     for name, row in selected[0]['reads'].items()}
            largest = {state: {name: {key: distribution([s['largest_states'][state][name][key] for s in selected])
                                     for key in row} for name, row in workloads.items()}
                       for state, workloads in selected[0]['largest_states'].items()}
            result[str(block)][name] = dict(metrics=metrics, classes=classes, reads=reads, largest_states=largest)
    return result


def display(summary):
    def text(value, digits=3):
        return f'{value["median"]:.{digits}f} [{value["minimum"]:.{digits}f},{value["maximum"]:.{digits}f}]'

    for block, configs in summary.items():
        print(f'B={block}: median [min,max], clean samples only', flush=True)
        for name, record in configs.items():
            metrics = record['metrics']
            print(f'  {name}: n={metrics["admission_ms"]["n"]}', flush=True)
            for metric, value in metrics.items():
                print(f'    {metric}: {text(value, 0 if metric.endswith("bytes") else 3)}', flush=True)
            for workload, row in record['reads'].items():
                print(f'    read {workload}: ' + ' '.join(f'{key}={text(value)}' for key, value in row.items()), flush=True)
            for state, workloads in record['largest_states'].items():
                for workload, row in workloads.items():
                    print(f'    state {state} {workload}: ' + ' '.join(f'{key}={text(value)}' for key, value in row.items()), flush=True)
            for basis, buckets in record['classes'].items():
                print(f'    admission classes by {basis}:', flush=True)
                for label, row in buckets.items():
                    phases = ' '.join(f'{key}={text(row[key])}' for key in
                        ('sort_or_fresh_ms', 'subtract_ms', 'seal_adopt_ms', 'merge_ms', 'admission_ms'))
                    print(f'      {label}: count={text(row["admissions"], 0)} {phases}', flush=True)


def parse_blocks(value):
    try:
        blocks = [int(part) for part in value.split(',')]
    except ValueError as error:
        raise argparse.ArgumentTypeError('blocks must be comma-separated integers') from error
    if not blocks or any(block <= 0 for block in blocks) or len(set(blocks)) != len(blocks):
        raise argparse.ArgumentTypeError('blocks must be distinct positive integers')
    return blocks


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--results', type=Path, required=True)
    parser.add_argument('--trace-archive', type=Path, required=True)
    parser.add_argument('--reps', type=int, default=3)
    parser.add_argument('--blocks', type=parse_blocks, default=[256])
    args = parser.parse_args()
    if args.reps < 3:
        parser.error('--reps must be at least 3 for median/range measurements')
    if not os.environ.get('BUNDLE_DIR'):
        parser.error('BUNDLE_DIR must identify the packaged bundle')
    bundle = Path(os.environ['BUNDLE_DIR']).resolve()
    manifest = json.loads((bundle / 'manifest.json').read_text())
    binary = bundle / manifest['binaries']['packed-replay']
    cwd = bundle / 'source' / 'experiments' / 'gpu'
    if not binary.is_file() or not cwd.is_dir():
        raise FileNotFoundError('bundle must contain its packed-replay binary and source snapshot')
    env = {k: v for k, v in os.environ.items() if not k.startswith('BENCH_CHECK_')}
    env['LD_LIBRARY_PATH'] = f'{bundle / "lib"}:/usr/local/nvidia/lib64:/usr/local/nvidia/lib'
    env['PATH'] = f'{bundle / "bin"}:' + env.get('PATH', '/usr/bin:/bin')
    env.pop('CUDA_HOME', None)
    args.results = args.results.resolve()
    args.results.mkdir(parents=True, exist_ok=True)
    label = manifest['commit'][:12]
    destination = args.results / f'packed-replay-{label}.json'
    if destination.exists():
        raise FileExistsError(f'refusing to overwrite previous results: {destination}')
    data = dict(commit=manifest['commit'], bundle=str(bundle), repetitions=args.reps,
                blocks=args.blocks, trace_archive=str(args.trace_archive.resolve()),
                protocol='fresh process per configuration; paired replay; selected column per configuration',
                configurations={name: dict(flags=list(flags), report_prefix=prefix, storage=storage)
                                for name, flags, prefix, storage in CONFIGURATIONS},
                samples=[], summary={}, complete=False)

    def save():
        temporary = destination.with_suffix('.tmp')
        temporary.write_text(json.dumps(data, indent=2) + '\n')
        temporary.replace(destination)

    try:
        with Path('/tmp/mini_linq-gpu.lock').open('a') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            before = tenants(env)
            if before:
                raise RuntimeError('foreign GPU tenant before extraction: ' + before)
            TRACE.parent.mkdir(parents=True, exist_ok=True)
            # The supplied archive has a trace/ top-level directory. No old
            # result is used as an extraction fallback, and tar failures abort.
            subprocess.run(['tar', '--zstd', '--no-same-owner', '-xf', str(args.trace_archive.resolve()),
                            '-C', str(TRACE.parent)], env=env, check=True)
            trace = json.loads((TRACE / 'manifest.json').read_text())
            expected_rows = sum(state['final_rows'] for state in trace['states'])
            data['trace_source'] = trace['source']
            data['nvidia_smi'] = subprocess.check_output(['nvidia-smi'], env=env, text=True, timeout=10)
            save()
            for repetition in range(args.reps):
                block_order = args.blocks[repetition % len(args.blocks):] + args.blocks[:repetition % len(args.blocks)]
                for block_index, block in enumerate(block_order):
                    shift = (repetition + block_index) % len(CONFIGURATIONS)
                    order = CONFIGURATIONS[shift:] + CONFIGURATIONS[:shift]
                    for name, flags, prefix, storage in order:
                        output = args.results / f'{label}-b{block}-{name}-{repetition + 1}.json'
                        log = output.with_suffix('.log')
                        if output.exists() or log.exists():
                            raise FileExistsError(f'refusing to overwrite a sample: {output}')
                        before = tenants(env)
                        if before:
                            raise RuntimeError('foreign GPU tenant before replay: ' + before)
                        argv = [str(binary), str(TRACE), '--block', str(block), '--output', str(output), *flags]
                        record = dict(configuration=name, report_prefix=prefix, block_rows=block,
                                      repetition=repetition + 1, argv=argv, report=str(output), log=str(log),
                                      tenants_before=before, exact=False)
                        data['samples'].append(record)
                        save()
                        started = time.monotonic()
                        try:
                            with log.open('w') as stream:
                                proc = subprocess.run(argv, cwd=cwd, env=env, stdout=stream,
                                                      stderr=subprocess.STDOUT, timeout=300)
                            record['exit_code'] = proc.returncode
                        finally:
                            record['process_seconds'] = time.monotonic() - started
                            record['tenants_after'] = tenants(env)
                            save()
                        if record['tenants_after']:
                            raise RuntimeError('foreign GPU tenant after replay: ' + record['tenants_after'])
                        if proc.returncode != 0:
                            raise RuntimeError(f'{name} B={block}: oracle/replay failure, exit {proc.returncode}; see {log}')
                        report = json.loads(output.read_text())
                        if report['timing_indicative'] is not False:
                            raise RuntimeError(f'{name} B={block}: replay observed a foreign GPU tenant')
                        if report['storage'] != storage or report['block_rows'] != block:
                            raise RuntimeError(f'{name}: replay configuration mismatch')
                        if report['totals']['rows'] != expected_rows or len(report['states']) != len(trace['states']):
                            raise RuntimeError(f'{name}: incomplete trace replay')
                        # Successful replay publishes JSON only after all three
                        # subtraction/content/prefix oracles have passed.
                        record['measurements'] = measurements(report, prefix)
                        record['exact'] = True
                        save()
                        print(f'{label} B={block} {name} repetition={repetition + 1}: '
                              f'{record["measurements"]["metrics"]["admission_ms"]:.3f} ms admission', flush=True)
            data['summary'] = summarize(data['samples'])
            data['complete'] = True
            save()
            display(data['summary'])
        return 0
    except Exception as error:
        data['error'] = f'{type(error).__name__}: {error}'
        save()
        print(data['error'], file=sys.stderr, flush=True)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
