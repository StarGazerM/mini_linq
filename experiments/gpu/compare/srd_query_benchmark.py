#!/usr/bin/env python3
"""Reproduce the SRDatalog DOOP query comparison, without compiling during runs.

From the MiniLinq checkout (all paths have CLI overrides)::

    python3 experiments/gpu/compare/srd_query_benchmark.py build
    python3 experiments/gpu/compare/srd_query_benchmark.py run
    python3 experiments/gpu/compare/srd_query_benchmark.py export

``build`` compiles only SRDatalog, with no deadline, using the checkout's .venv.
Prebuild srd-doop-mini, srd-doop-mini-par, srd-doop-gpu and doop-gpu separately.
``run`` uses one fresh-process warmup and three measurements, each killed after
10 seconds. ``export`` performs separate correctness runs (default 300 seconds,
override with --correctness-timeout); these never enter benchmark medians.
``run --export-tuples`` does both. Select subsets using --engines and --datasets;
sequential invocations accumulate records in --results. Do not write the same
results file from concurrent CPU/GPU invocations. No timeout is interpreted as
a count or equality result. Exports are retained. Serial ``mini`` is included
by default for correctness, not timing. Existing serial exports can be checked::

    python3 experiments/gpu/compare/srd_query_benchmark.py compare --dataset xalan \\
        --mini-export /path/to/mini --mini-log /path/to/mini.stdout.log \\
        --srd-report /path/to/field_flow/native.json

The comparison is written to --report (default WORK/DATASET-external-comparison.json)
and appended to --results; original external process exit evidence stays external.

All SRD builds use plan=baseline, including the research candidate.program
field-flow rewrite. No query rules or planner choices are changed here. Exact
comparison externally sorts canonical integer rows, then compares every byte;
hashes are provenance, not a substitute for equality. Field-flow comparisons
require all 43 IDBs; default-vs-field-flow checks explicitly cover only the
original IDBs (the default query does not define the three new auxiliaries).

Prerequisites: source revision fab849f, its Python dependencies and vendored
CUDA/Boost headers, clang++/ccache/ninja toolchain, nvidia-smi, GNU sort, enough
disk for retained exports and external-sort scratch, and an idle GPU. The lock
serializes this orchestrator; all other GPU users must honor the same lock or
stay idle. No ncu is used. SM samples are device-wide, not kernel-specific.
"""
from __future__ import annotations

import argparse
import contextlib
import datetime as dt
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
import statistics
import subprocess
import sys
import tempfile
import threading
import time

REPO = Path(__file__).resolve().parents[3]
SOURCE = Path('/home/stargazermiao/workspace/srdatalog-python')
DATA = Path('/data/doop-bench')
ENGINES = ('mini-par', 'mini-gpu', 'srd-default', 'srd-field_flow', 'original-doop-gpu')
TIMEOUT = 10.0
CANDIDATE = 'research/doop_field_flow/harness/candidate.py'
ENV_KEYS = ('PATH', 'LD_LIBRARY_PATH', 'CUDA_HOME', 'CUDA_PATH', 'CXX', 'CC',
            'CARGO_TARGET_DIR', 'RUSTFLAGS', 'SRDATALOG_RMM_RESOURCE',
            'GPU_DATALOG_PHASES', 'RAYON_NUM_THREADS', 'CUDA_VISIBLE_DEVICES')


def load(path):
    return json.loads(Path(path).read_text())


def save(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + '.tmp')
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')
    temporary.replace(path)


def sha(path):
    digest = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def capture(command, cwd=None):
    return subprocess.check_output(command, cwd=cwd, text=True).strip()


def identity(source):
    revision = capture(['git', 'rev-parse', 'HEAD'], source)
    if not revision.startswith('fab849f'):
        raise ValueError(f'Expected SRDatalog fab849f, found {revision}')
    return {'revision': revision,
            'tracked_diff_sha256': hashlib.sha256(subprocess.check_output(
                ['git', 'diff', 'HEAD', '--'], cwd=source)).hexdigest(),
            'files': {name: sha(source / name) for name in
                      ('examples/doop.py', 'examples/doop_suite/gpu.py', CANDIDATE)}}


def environment():
    result = os.environ.copy()
    result.update(SRDATALOG_RMM_RESOURCE='cuda_async', GPU_DATALOG_PHASES='1',
                  RAYON_NUM_THREADS='15', PYTHONDONTWRITEBYTECODE='1')
    return result


def worker(args):
    # Import from the exact source checkout, not the driver's historical frozen
    # harness/source directory or an installed SRDatalog distribution.
    sys.path[:0] = [str(args.source / 'src'), str(args.source / 'examples')]
    os.environ['SRDATALOG_RMM_RESOURCE'] = 'cuda_async'
    from doop_suite import gpu
    build_path = args.work / 'builds' / args.dataset / args.variant / 'build.json'
    facts = args.prepared / args.dataset
    if args.command == '_build':
        if args.variant == 'field_flow':
            spec = importlib.util.spec_from_file_location('srd_field_flow_candidate', args.source / CANDIDATE)
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            gpu._program = module.program
        build_path.parent.mkdir(parents=True, exist_ok=True)
        before = identity(args.source)
        gpu._build(facts, build_path.parent, 'baseline', args.jobs)
        after = identity(args.source)
        if before != after:
            raise RuntimeError('SRDatalog source changed during compilation')
        build = load(build_path)
        build.update(variant=args.variant, plan='baseline', source_identity=before,
                     facts=str(facts), dataset=args.dataset)
        save(build_path, build)
    else:
        build = load(build_path)
        if sha(build['library']) != build['library_sha256']:
            raise ValueError('Built SRDatalog library checksum mismatch')
        if sha(facts / 'manifest.json') != build['input_manifest_sha256']:
            raise ValueError('Prepared manifest differs from build')
        gpu._execute(build, facts, args.report, args.export_dir)


def no_gpu_users():
    users = capture(['nvidia-smi', '--query-compute-apps=pid,process_name',
                     '--format=csv,noheader'])
    if users:
        raise RuntimeError('GPU compute processes must be stopped first: ' + users)


def monitored(command, directory, cwd, env, *, gpu=True, timeout=TIMEOUT):
    """Gate success on process exit; preserve partial logs on SIGKILL timeout."""
    directory.mkdir(parents=True, exist_ok=False)
    if gpu:
        no_gpu_users()
    record = {'command': list(map(str, command)), 'cwd': str(cwd),
              'environment': {k: env[k] for k in ENV_KEYS if k in env},
              'started_utc': dt.datetime.now(dt.timezone.utc).isoformat(),
              'uptime_before': capture(['uptime']),
              'proc_uptime_before': Path('/proc/uptime').read_text().strip(),
              'timeout_seconds': timeout, 'sm_clock_samples': [],
              'clock_sampling': 'nvidia-smi -lms 250; device-wide SM MHz',
              'stdout': str(directory / 'stdout.log'), 'stderr': str(directory / 'stderr.log')}
    def sample(monitor_process):
        for line in monitor_process.stdout:
            record['sm_clock_samples'].append({
                'process_elapsed_seconds': time.monotonic() - began,
                'csv': line.strip()})

    with (directory / 'stdout.log').open('xb') as out, (directory / 'stderr.log').open('xb') as err:
        began = time.monotonic()
        child = subprocess.Popen(command, cwd=cwd, env=env, stdout=out, stderr=err,
                                 start_new_session=True)
        clock_process = subprocess.Popen(
            ['nvidia-smi', '--query-gpu=index,uuid,timestamp,clocks.sm,utilization.gpu,memory.used',
             '--format=csv,noheader,nounits', '-lms', '250'],
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        monitor = threading.Thread(target=sample, args=(clock_process,), daemon=True)
        monitor.start()
        try:
            child.wait(timeout=max(0, timeout - (time.monotonic() - began)))
            record['status'] = 'passed' if child.returncode == 0 else 'failed'
        except subprocess.TimeoutExpired:
            record['status'] = 'timeout'
            with contextlib.suppress(ProcessLookupError):
                os.killpg(child.pid, signal.SIGKILL)
            child.wait()
        except BaseException:
            with contextlib.suppress(ProcessLookupError):
                os.killpg(child.pid, signal.SIGKILL)
            child.wait()
            raise
        finally:
            record['process_seconds'] = time.monotonic() - began
            record['returncode'] = child.returncode
            clock_process.terminate()
            clock_process.wait()
            monitor.join()
    record['uptime_after'] = capture(['uptime'])
    record['proc_uptime_after'] = Path('/proc/uptime').read_text().strip()
    return record


def parse_logs(record):
    counts, runtime, phases, cpu_rounds = {}, {}, [], []
    for path in (record['stdout'], record['stderr']):
        for line in Path(path).read_text(errors='replace').splitlines():
            if line.startswith('counts\t'):
                counts = json.loads(line.split('\t', 1)[1])
            elif line.startswith('runtime\t'):
                runtime = json.loads(line.split('\t', 1)[1])
            elif line.startswith('phase\t'):
                phases.append(line.split('\t')[1:])
            elif line.startswith('datalog-phase '):
                cpu_rounds.append({k: int(v) for k, v in re.findall(r'(\w+_ns)=(\d+)', line)})
            elif re.fullmatch(r'\w+\t\d+', line):
                name, count = line.split('\t')
                counts[name] = int(count)
            elif line.startswith('gpu_evaluation_seconds\t'):
                runtime['evaluation_seconds'] = float(line.split('\t')[1])
    record.update(relation_counts=counts, runtime=runtime, phases=phases,
                  cpu_round_phases_ns=cpu_rounds)
    record['gpu_phases_seconds'] = {
        phase[0]: float(phase[1]) for phase in phases
        if len(phase) == 2 and phase[0] in ('setup', 'fixpoint', 'output')
    }
    if cpu_rounds:
        record['cpu_round_phase_sums_ns'] = {
            k: sum(item.get(k, 0) for item in cpu_rounds)
            for k in ('evaluation_ns', 'admission_ns', 'publication_ns')}
        record['cpu_phase_scope'] = 'sum of round phases; NOT whole-program fixedpoint wall time'
    record['evaluation_scope'] = 'harness evaluation wall time, includes output materialization; excludes load/export'


def worker_command(args, mode, dataset, variant):
    return [str(args.python), '-B', str(Path(__file__).resolve()), mode,
            '--source', str(args.source), '--prepared', str(args.prepared),
            '--work', str(args.work), '--dataset', dataset, '--variant', variant,
            '--jobs', str(args.jobs)]


def builds(args):
    source = identity(args.source)
    for dataset in args.datasets:
        if dataset == 'cassandra':
            continue
        for engine in args.engines:
            if not engine.startswith('srd-'):
                continue
            variant = engine.removeprefix('srd-')
            directory = args.work / 'builds' / dataset / variant
            directory.mkdir(parents=True, exist_ok=True)
            command = worker_command(args, '_build', dataset, variant)
            start = time.monotonic()
            with (directory / 'build.log').open('ab') as log:
                result = subprocess.run(command, cwd=args.source, env=environment(),
                                        stdout=log, stderr=subprocess.STDOUT)
            save(directory / 'build-process.json', {
                'command': command, 'returncode': result.returncode,
                'process_seconds': time.monotonic() - start, 'timeout_seconds': None,
                'source_identity': source,
                'environment': {k: environment()[k] for k in ENV_KEYS if k in environment()}})
            if result.returncode:
                raise RuntimeError(f'Compilation failed: {directory / "build.log"}')


def schema(args, dataset, variant='field_flow'):
    build = load(args.work / 'builds' / dataset / variant / 'build.json')
    relations = {r['name']: r['arity'] for r in build['relations'] if not r['input_file']}
    if variant == 'field_flow' and len(relations) != 43:
        raise ValueError(f'Expected all 43 field-flow IDBs, found {len(relations)}')
    return relations


def count_schema(args, record):
    variant = 'default' if record['engine'] == 'srd-default' else 'field_flow'
    path = args.work / 'builds' / record['dataset'] / variant / 'build.json'
    if path.exists():
        return set(schema(args, record['dataset'], variant)), 'built IDB schema'
    if record['engine'] in ('mini', 'mini-par', 'mini-gpu'):
        # CPU-only collection may precede SRD compilation. Exact name/arity
        # validation happens when comparing against the completed SRD build.
        names = set(record['relation_counts'])
        if len(names) != 43:
            raise ValueError(f"Expected 43 field-flow IDB counts, found {len(names)}")
        return names, '43 reported IDBs; names/arity await SRD build comparison'
    raise FileNotFoundError(path)


def launch(args, engine, dataset, directory, export=False):
    exports = directory / 'relations' if export else None
    native = directory / 'native.json'
    if engine.startswith('srd-'):
        variant = engine.removeprefix('srd-')
        build = load(args.work / 'builds' / dataset / variant / 'build.json')
        if build['source_identity'] != identity(args.source):
            raise ValueError('SRD build source identity differs from current source')
        command = worker_command(args, '_run', dataset, variant) + ['--report', str(native)]
        if export:
            command += ['--export-dir', str(exports)]
    elif engine == 'original-doop-gpu':
        command = [str(args.original_gpu), str(args.original_data / dataset), '--gpu-only']
    else:
        binary = args.mini if engine == 'mini' else args.mini_par if engine == 'mini-par' else args.mini_gpu
        command = [str(binary), str(args.prepared / dataset)]
        if engine == 'mini-par':
            command += ['--threads', '15']
        elif engine == 'mini-gpu':
            command += ['--gpu-only']
        if export:
            command += ['--export', str(exports)]
    record = monitored(command, directory, args.repo, environment(),
                       gpu=engine not in ('mini', 'mini-par'),
                       timeout=args.correctness_timeout if export else TIMEOUT)
    record['executable_sha256'] = sha(command[0])
    if engine.startswith('srd-'):
        record['build'] = build
    record.update(engine=engine, dataset=dataset, purpose='correctness' if export else 'benchmark')
    parse_logs(record)
    if native.exists():
        record['native'] = load(native)
        record['relation_counts'] = record['native'].get('relation_counts', {})
        if record['status'] == 'passed' and record['native'].get('status') != 'native_completed':
            record['status'] = 'failed'
    elif engine.startswith('srd-') and record['status'] == 'passed':
        record['status'] = 'failed'
        record['error'] = 'No completed native report'
    if exports is not None:
        record['exports'] = str(exports)
    save(directory / 'process.json', record)
    return record


def exact_relation(left, right, arity, left_count, right_count, scratch):
    """Bounded RAM, integer-normalized TSV, exact sorted stream comparison."""
    identities = []
    with tempfile.TemporaryDirectory(dir=scratch, prefix='tuple-sort-') as tmp:
        sorted_paths = []
        for index, path in enumerate((left, right)):
            canonical = Path(tmp) / f'{index}.tsv'
            rows = 0
            with Path(path).open() as source, canonical.open('w') as target:
                for line in source:
                    fields = line.rstrip('\n\r').split('\t')
                    if len(fields) != arity:
                        raise ValueError(f'{path}: row {rows + 1}: expected arity {arity}')
                    target.write('\t'.join(str(int(x)) for x in fields) + '\n')
                    rows += 1
            ordered = Path(tmp) / f'{index}.sorted'
            subprocess.run(['sort', '--parallel=1', '-S', '256M', '-T', tmp,
                            '-o', str(ordered), str(canonical)],
                           env={**os.environ, 'LC_ALL': 'C'}, check=True)
            unique = 0
            previous = None
            with ordered.open('rb') as source:
                for line in source:
                    if line != previous:
                        unique += 1
                    previous = line
            identities.append({'path': str(path), 'sha256': sha(path), 'rows': rows,
                               'unique_rows': unique, 'sorted_sha256': sha(ordered)})
            sorted_paths.append(ordered)
        equal = True
        with sorted_paths[0].open('rb') as lhs, sorted_paths[1].open('rb') as rhs:
            while True:
                a, b = lhs.read(1024 * 1024), rhs.read(1024 * 1024)
                if a != b:
                    equal = False
                    break
                if not a:
                    break
        counts_ok = all(item['rows'] == item['unique_rows'] == expected
                        for item, expected in zip(identities, (left_count, right_count)))
        return {'passed': equal and counts_ok, 'exact_bytes_equal_after_integer_sort': equal,
                'export_counts_match_json_and_no_duplicates': counts_ok,
                'expected_json_counts': [left_count, right_count], 'files': identities}


def compare_records(args, left, right, relations):
    result = {'left': left['engine'], 'right': right['engine'], 'dataset': left['dataset'],
              'scope': 'all declared IDBs of comparison schema', 'relations': {}}
    if left['status'] != 'passed' or right['status'] != 'passed':
        return {**result, 'status': 'unavailable', 'reason': 'export process did not complete normally'}
    for record in (left, right):
        expected = schema(args, record['dataset'],
                          'default' if record['engine'] == 'srd-default' else 'field_flow')
        suffix = '.tsv' if record['engine'].startswith('srd-') else '.csv'
        found = {p.stem for p in Path(record['exports']).glob('*' + suffix)}
        if found != set(expected):
            return {**result, 'status': 'failed', 'error': 'incomplete/extra IDB export schema',
                    'engine': record['engine'], 'missing': sorted(set(expected) - found),
                    'unexpected': sorted(found - set(expected))}
    for name, arity in relations.items():
        paths = [Path(r['exports']) / (name + ('.tsv' if r['engine'].startswith('srd-') else '.csv'))
                 for r in (left, right)]
        result['relations'][name] = exact_relation(
            *paths, arity, left['relation_counts'][name], right['relation_counts'][name], args.work)
    result['status'] = 'passed' if all(r['passed'] for r in result['relations'].values()) else 'failed'
    return result


def summaries(records):
    result = []
    keys = sorted({(r['dataset'], r['engine']) for r in records})
    for dataset, engine in keys:
        measured = [r for r in records if r['dataset'] == dataset and r['engine'] == engine
                    and r['purpose'] == 'benchmark' and not r['warmup']]
        if not measured:
            continue
        good = [r for r in measured if r['status'] == 'passed']
        def median(values):
            return statistics.median(values) if values else None
        result.append({'dataset': dataset, 'engine': engine,
                       'completed_repetitions': len(good), 'attempted_repetitions': len(measured),
                       'statuses': [r['status'] for r in measured],
                       'median_process_seconds_completed_only': median([r['process_seconds'] for r in good]),
                       'median_evaluation_seconds_completed_only': median([
                           r['runtime']['evaluation_seconds'] for r in good if 'evaluation_seconds' in r['runtime']]),
                       'median_fixedpoint_seconds_completed_only': median([
                           r['native']['fixedpoint_seconds'] if 'fixedpoint_seconds' in r.get('native', {})
                           else r['gpu_phases_seconds']['fixpoint']
                           for r in good if 'fixedpoint_seconds' in r.get('native', {})
                           or 'fixpoint' in r.get('gpu_phases_seconds', {})]),
                       'median_gpu_phases_seconds_completed_only': {
                           phase: median([r['gpu_phases_seconds'][phase] for r in good
                                          if phase in r.get('gpu_phases_seconds', {})])
                           for phase in ('setup', 'fixpoint', 'output')}})
    return result


def run_suite(args):
    source = identity(args.source)
    result = load(args.results) if args.results.exists() else {
        'schema_version': 1, 'runs': [], 'comparisons': [], 'source_identity': source,
        'policy': {'warmups': 1, 'repeats': 3, 'process_timeout_seconds': TIMEOUT,
                   'compilation_timeout_seconds': None, 'threads': 15,
                   'plan': 'baseline', 'memory_resource': 'cuda_async',
                   'timeout_scope': 'entire fresh process including normal teardown, and export when requested',
                   'exports': 'separate correctness processes, never part of measured medians'}}
    if result['source_identity'] != source:
        raise ValueError('Existing results belong to a different SRDatalog source identity')
    result['orchestrator_sha256'] = sha(__file__)
    result['mini_revision'] = capture(['git', 'rev-parse', 'HEAD'], args.repo)
    result['status'] = 'running'
    session = args.work / 'runs' / dt.datetime.now(dt.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    result.setdefault('sessions', []).append({'directory': str(session), 'argv': sys.argv,
                                            'nvidia_smi': capture(['nvidia-smi'])})
    # Fingerprint all prepared files once, outside bounded timed runs.
    inputs = {
        ds: {str(p.relative_to(args.prepared / ds)): sha(p)
             for p in sorted((args.prepared / ds).iterdir()) if p.is_file()}
        for ds in args.datasets if ds != 'cassandra'}
    previous = result.setdefault('prepared_inputs', {})
    for dataset, files in inputs.items():
        if dataset in previous and previous[dataset] != files:
            raise ValueError(f'Prepared inputs changed since previous runs: {dataset}')
        previous[dataset] = files
    save(args.results, result)
    try:
        for dataset in args.datasets:
            engines = [e for e in args.engines if dataset != 'cassandra' or e == 'original-doop-gpu']
            if args.command == 'run':
                timed_engines = [e for e in engines if e != 'mini']
                for index in range(-1, 3):
                    # Alternate order, as in the retained research paired harness.
                    for engine in timed_engines if index % 2 else reversed(timed_engines):
                        record = launch(args, engine, dataset, session / dataset / engine / f'run-{index}')
                        record.update(index=index, warmup=index < 0)
                        result['runs'].append(record)
                        result['summary'] = summaries(result['runs'])
                        save(args.results, result)
            if args.command == 'export' or args.export_tuples:
                for engine in engines:
                    if engine == 'original-doop-gpu':
                        continue  # Different query: never assert tuple equivalence.
                    record = launch(args, engine, dataset, session / dataset / engine / 'export', True)
                    result['runs'].append(record)
                    save(args.results, result)
                available = {r['engine']: r for r in result['runs']
                             if r['dataset'] == dataset and r['purpose'] == 'correctness'}
                oracle = available.get('srd-field_flow')
                if oracle:
                    for engine in ('mini', 'mini-par', 'mini-gpu', 'srd-default'):
                        if engine in available:
                            relations = schema(args, dataset, 'default' if engine == 'srd-default' else 'field_flow')
                            comparison = compare_records(args, available[engine], oracle, relations)
                            result['comparisons'].append(comparison)
                            save(args.results, result)
                if 'mini' in available and 'mini-gpu' in available:
                    result['comparisons'].append(compare_records(
                        args, available['mini'], available['mini-gpu'], schema(args, dataset)))
                    save(args.results, result)
        # Counts are checked per relation, not only VarPointsTo, and only for
        # normally exited processes. The default query has fewer auxiliaries.
        checks = []
        for dataset in args.datasets:
            complete = [r for r in result['runs'] if r['dataset'] == dataset
                        and r['status'] == 'passed' and r['engine'] != 'original-doop-gpu']
            for record in complete:
                expected, schema_scope = count_schema(args, record)
                own = record['relation_counts']
                checks.append({'dataset': dataset, 'engine': record['engine'],
                               'check': 'all IDB counts present', 'schema_scope': schema_scope,
                               'passed': expected <= set(own)})
                for other in complete:
                    if other is record:
                        break
                    common = expected & count_schema(args, other)[0]
                    mismatches = {n: [own.get(n), other['relation_counts'].get(n)] for n in common
                                  if own.get(n) != other['relation_counts'].get(n)}
                    checks.append({'dataset': dataset, 'engine': record['engine'],
                                   'other_engine': other['engine'], 'check': 'shared IDB counts',
                                   'relations_compared': len(common), 'passed': not mismatches,
                                   'mismatches': mismatches})
        result['count_checks'] = checks
        result['summary'] = summaries(result['runs'])
        failures = any(r['status'] != 'passed' for r in result['runs'])
        failures |= any(not c['passed'] for c in checks)
        failures |= any(c['status'] != 'passed' for c in result['comparisons'])
        result['status'] = 'completed_with_failures_or_timeouts' if failures else 'completed'
        result['tuple_validation'] = 'not_requested' if not result['comparisons'] else 'see_comparisons'
    except BaseException as error:
        result.update(status='failed', error=repr(error))
        raise
    finally:
        save(args.results, result)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('command', choices=('build', 'run', 'export', 'compare', '_build', '_run'))
    parser.add_argument('--repo', type=Path, default=REPO)
    parser.add_argument('--source', type=Path, default=SOURCE)
    parser.add_argument('--python', type=Path)
    parser.add_argument('--prepared', type=Path, default=DATA / 'doop-vs-srdatalog-20261026/fresh-corpus/prepared')
    parser.add_argument('--original-data', type=Path, default=DATA / 'doop-data')
    parser.add_argument('--work', type=Path, default=DATA / 'srd-query-xalan-batik')
    parser.add_argument('--results', type=Path)
    parser.add_argument('--mini', type=Path)
    parser.add_argument('--mini-export', type=Path, help='compare: existing serial MiniLinq export directory')
    parser.add_argument('--mini-log', type=Path, help='compare: existing counts/runtime stdout log')
    parser.add_argument('--srd-report', type=Path, help='compare: existing native JSON with outputs and counts')
    parser.add_argument('--mini-par', type=Path)
    parser.add_argument('--mini-gpu', type=Path)
    parser.add_argument('--original-gpu', type=Path)
    parser.add_argument('--engines', nargs='+', choices=('mini', *ENGINES), default=None)
    parser.add_argument('--datasets', nargs='+', choices=('xalan', 'batik', 'cassandra'),
                        default=['xalan', 'batik', 'cassandra'])
    parser.add_argument('--jobs', type=int, default=2)
    parser.add_argument('--export-tuples', action='store_true')
    parser.add_argument('--correctness-timeout', type=float, default=300.0,
                        help='export-only deadline; benchmark deadline is always 10s')
    parser.add_argument('--gpu-lock', type=Path, default=Path('/tmp/srd-query-benchmark.gpu.lock'))
    parser.add_argument('--dataset', choices=('xalan', 'batik'))
    parser.add_argument('--variant', choices=('default', 'field_flow'))
    parser.add_argument('--report', type=Path)
    parser.add_argument('--export-dir', type=Path)
    args = parser.parse_args()
    if args.engines is None:
        args.engines = list(ENGINES) + (['mini'] if args.command == 'export' or args.export_tuples else [])
    if args.correctness_timeout <= 0:
        parser.error('--correctness-timeout must be positive')
    for name in ('repo', 'source', 'prepared', 'original_data', 'work'):
        setattr(args, name, getattr(args, name).resolve())
    args.python = args.python or args.source / '.venv/bin/python'
    args.results = args.results or args.repo / 'experiments/gpu/compare/srd_query_xalan_batik.results.json'
    args.mini = args.mini or args.repo / 'features/target/release/srd-doop-mini'
    args.mini_par = args.mini_par or args.repo / 'features/target/release/srd-doop-mini-par'
    args.mini_gpu = args.mini_gpu or args.repo / 'experiments/gpu/target/release/srd-doop-gpu'
    args.original_gpu = args.original_gpu or args.repo / 'experiments/gpu/target/release/doop-gpu'
    for name in ('python', 'results', 'mini', 'mini_par', 'mini_gpu', 'original_gpu'):
        # Do not resolve Python symlinks: doing so bypasses virtualenv discovery.
        setattr(args, name, Path(os.path.abspath(getattr(args, name))))
    if args.command.startswith('_'):
        worker(args)
        return
    args.work.mkdir(parents=True, exist_ok=True)
    if args.command == 'build':
        builds(args)
        return
    if args.command == 'compare':
        if not all((args.dataset, args.mini_export, args.mini_log, args.srd_report)):
            parser.error('compare requires --dataset --mini-export --mini-log --srd-report')
        native = load(args.srd_report)
        left = {'engine': 'mini', 'dataset': args.dataset, 'status': 'passed',
                'exports': str(args.mini_export), 'stdout': str(args.mini_log),
                'stderr': str(args.mini_log)}
        parse_logs(left)
        paths = {str(Path(p).parent) for p in native['outputs'].values()}
        if len(paths) != 1:
            raise ValueError('SRD report must identify one complete export directory')
        right = {'engine': 'srd-field_flow', 'dataset': args.dataset,
                 'status': 'passed' if native['status'] == 'native_completed' else 'failed',
                 'exports': paths.pop(), 'relation_counts': native['relation_counts']}
        comparison = compare_records(args, left, right, schema(args, args.dataset))
        comparison['process_exit_evidence'] = 'external artifacts; caller must separately retain process exit/deadline evidence'
        save(args.report or args.work / f'{args.dataset}-external-comparison.json', comparison)
        combined = load(args.results) if args.results.exists() else {
            'schema_version': 1, 'runs': [], 'comparisons': [],
            'source_identity': identity(args.source), 'status': 'comparison_only'}
        combined['comparisons'].append(comparison)
        combined['tuple_validation'] = 'see_comparisons'
        save(args.results, combined)
        if comparison['status'] != 'passed':
            raise RuntimeError('Exact tuple comparison failed or unavailable')
        return
    if set(args.engines) <= {'mini', 'mini-par'}:
        run_suite(args)
        print(args.results)
        return
    with args.gpu_lock.open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        run_suite(args)
    print(args.results)


if __name__ == '__main__':
    main()
