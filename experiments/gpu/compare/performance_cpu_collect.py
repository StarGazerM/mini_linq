#!/usr/bin/env python3
"""Collect existing CPU profiles or isolated, exact plain-only par t1/t24 runs.

Parallel protocol (run serially): all --par --threads 1|24 --sf 0.1|1|10
--cap-seconds 100. SF0.1 verifies one untimed answer; SF1/SF10 use one warmup
and five timed runs. Artifacts use performance_cpu_par_t{threads}_sf{sf}
and performance_cpu_raw/par/t{threads}/sf{sf}; existing CPU data is untouched.
"""
import argparse
import csv
from functools import cache
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'experiments/gpu/compare'
CACHE_ROOT = Path('/tmp/mini-linq-performance-cpu')
CACHE = CACHE_ROOT
SF = 1
RUNS = 5
CAP_SECONDS = 600
SLOW_SECONDS = 60
PROFILE_WINNERS_ONLY = False
CONFIGS = ['default', 'le', 'le_runs', 'advised']
PAR = False
THREADS = None
DATABASE = None
WORK = ROOT / 'features/workloads'
SOURCE = ROOT / 'experiments/gpu/tpch/src/source.rs'
NAMES = ['retail_' + x for x in re.findall(r'"([a-z0-9_]+)"', SOURCE.read_text().split('sources!(', 1)[1].split(');', 1)[0])]
if 'retail_h14' not in NAMES:
    NAMES.insert(NAMES.index('retail_h15'), 'retail_h14')
CATALOG_NAMES = NAMES.copy()
RAW = OUT / 'performance_cpu_raw'
ENV = dict(os.environ, CARGO_BUILD_JOBS='1', CARGO_TARGET_DIR=str(CACHE / 'target'))


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + '\n')


def artifact_name(kind=''):
    prefix = f'performance_cpu_par_t{THREADS}' if PAR else 'performance_cpu'
    return f'{prefix}{("_" + kind) if kind else ""}_sf{SF}'


def crate_directory(name):
    return (CACHE_ROOT if PAR else CACHE) / 'crates' / name


@cache
def compiler_fingerprint():
    """Hash compiler/runtime inputs once per invocation, including new sources."""
    paths = [ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', ROOT / 'rust-toolchain.toml']
    for root in ['facade', 'features', 'languages', 'passes', 'runtime', 'testkit', '.cargo']:
        for directory, children, files in os.walk(ROOT / root):
            children[:] = sorted(child for child in children if child not in ['target', '.git'])
            paths.extend(Path(directory) / name for name in files
                         if name.endswith(('.rs', '.toml')) or name == 'Cargo.lock')
    digest = hashlib.sha256()
    for path in sorted(set(paths)):
        digest.update(str(path.relative_to(ROOT)).encode() + b'\0')
        digest.update(path.read_bytes())
        digest.update(b'\0')
    return digest.hexdigest()


def validate_raw_stamp(name, stamp, required=False):
    if not PAR:
        return
    path = RAW / name / 'source_stamp.json'
    recorded = json.loads(path.read_text()) if path.exists() else None
    if (required or (RAW / name / 'par').exists()) and recorded != stamp:
        raise RuntimeError(f'{RAW / name}: parallel artifacts have missing/stale source fingerprints; archive this query directory before recollecting')


def invoke(args, log, env=ENV, timeout=None):
    with log.open('a') as stream:
        stream.write('\nCOMMAND ' + json.dumps([str(x) for x in args]) + '\n')
        stream.flush()
        subprocess.run([str(x) for x in args], cwd=ROOT, env=env, stdout=stream, stderr=subprocess.STDOUT, check=True, timeout=timeout)


def build():
    dependencies = {
        'mini-linq': 'facade/mini-linq', 'cq-load': 'runtime/cq-load',
        'cq-storage': 'runtime/cq-storage', 'cq-planners': 'features/planners',
        'cq-rewrites': 'features/rewrites', 'cq-join-filters': 'features/join-filters',
        'cq-ext-aggregate': 'features/extensions/cq-ext-aggregate',
        'cq-ext-correlated': 'features/extensions/cq-ext-correlated',
    }
    CACHE.mkdir(parents=True, exist_ok=True)
    log = OUT / (artifact_name('build') + '.log') if PAR else OUT / 'performance_cpu_build.log'
    workers = [(None, 'timing-worker')]
    if not PAR:
        workers += [('counters', 'counting-worker'), ('profile', 'profile-worker')]
    for name in NAMES:
        stamp = build_stamp(name) if PAR else None
        validate_raw_stamp(name, stamp)
        if all((RAW / name / c / ('plain.json' if PAR else 'profile.json')).exists() or (RAW / name / c / 'status.json').exists() for c in CONFIGS):
            continue
        if stamp is None:
            stamp = build_stamp(name)
        crate = crate_directory(name)
        other = CACHE_ROOT / f'sf{10 if SF == 1 else 1}' / 'crates' / name
        if not PAR and not (crate / 'build_stamp.json').exists() and (other / 'build_stamp.json').exists() and json.loads((other / 'build_stamp.json').read_text()) == stamp:
            shutil.copytree(other, crate, dirs_exist_ok=True)
        if (crate / 'build_stamp.json').exists() and json.loads((crate / 'build_stamp.json').read_text()) == stamp and all(
            (crate / binary).exists() for _, binary in workers
        ):
            description = json.loads(subprocess.check_output([crate / 'timing-worker', 'describe']))
            assert set(CONFIGS) <= set(description['configs']), (name, description['configs'])
            save(RAW / name / 'description.json', description)
            if PAR:
                save(RAW / name / 'source_stamp.json', stamp)
            continue
        (crate / 'src').mkdir(parents=True, exist_ok=True)
        manifest = f'[package]\nname = "tpch-derived-{name}"\nversion = "0.0.0"\nedition = "2024"\n[workspace]\n[features]\ncounters = ["cq-storage/counters"]\nprofile = ["cq-storage/operator-timing"]\n[dependencies]\n'
        for dep, path in dependencies.items():
            features = ', features = ["parquet"]' if dep == 'cq-load' else ''
            manifest += f'{dep} = {{ path = "{ROOT / path}"{features} }}\n'
        manifest += 'serde_json = "1"\nrayon = "1"\n'
        (crate / 'Cargo.toml').write_text(manifest)
        main = '#![recursion_limit = "512"]\n'
        main += f'#[allow(unused_imports, unused_macros, dead_code)] #[path = "{WORK / "src/tpch_runtime.rs"}"] mod tpch_runtime;\n'
        main += f'#[path = "{WORK / "src/queries/retail/tpch" / (name + ".rs")}"] mod query;\n'
        main += f'#[path = "{WORK / "xtask/src/tpch/comparison.rs"}"] mod comparison;\n'
        main += f'include!("{CACHE_ROOT / "adaptive-worker.rs"}");\n'
        (crate / 'src/main.rs').write_text(main)
        shutil.copy(WORK / 'Cargo.lock', crate / 'Cargo.lock')
        invoke(['cargo', 'metadata', '--offline', '--format-version', '1', '--manifest-path', crate / 'Cargo.toml'], log)
        for feature, binary in workers:
            args = ['cargo', 'build', '--release', '--locked', '--offline', '--manifest-path', crate / 'Cargo.toml']
            if feature:
                args += ['--features', feature]
            invoke(args, log)
            shutil.copy(Path(ENV['CARGO_TARGET_DIR']) / 'release' / ('tpch-derived-' + name), crate / binary)
        description = json.loads(subprocess.check_output([crate / 'timing-worker', 'describe']))
        assert set(CONFIGS) <= set(description['configs']), (name, description['configs'])
        save(RAW / name / 'description.json', description)
        save(crate / 'build_stamp.json', stamp)
        if PAR:
            save(RAW / name / 'source_stamp.json', stamp)
        print('BUILT', name, description['configs'], flush=True)


def build_stamp(name):
    paths = [WORK / 'src/tpch_runtime.rs', WORK / 'xtask/src/tpch/worker.rs',
             WORK / 'xtask/src/tpch/comparison.rs', CACHE_ROOT / 'adaptive-worker.rs',
             WORK / 'src/queries/retail/tpch' / (name + '.rs'), Path(__file__)]
    stamp = {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
    if PAR:
        stamp['compiler_runtime_sha256'] = compiler_fingerprint()
    return stamp


def prepare():
    import duckdb
    database = str(DATABASE) if DATABASE is not None else f'/data/tpch-gpu/sf{SF}/dbgen.duckdb'
    db = duckdb.connect(database, read_only=True)
    db.execute('SET threads=1')
    for name in NAMES:
        if PAR:
            validate_raw_stamp(name, build_stamp(name), required=True)
        raw = RAW / name
        description = json.loads((raw / 'description.json').read_text())
        data = CACHE / 'data' / name
        data.mkdir(parents=True, exist_ok=True)
        for table, columns in description['exports']:
            target = data / (table + '.parquet')
            if not target.exists():
                db.execute(f"COPY (SELECT {columns} FROM {table}) TO '{target}' (FORMAT PARQUET, COMPRESSION SNAPPY, ROW_GROUP_SIZE 2048)")
        result = db.execute(description['sql'])
        decimals = [i for i, col in enumerate(result.description) if str(col[1]).startswith('DECIMAL')]
        rows = []
        for values in result.fetchall():
            row = []
            for i, v in enumerate(values):
                s = 'NULL' if v is None else str(v)
                if i in decimals and '.' in s:
                    s = s.rstrip('0').rstrip('.')
                    if s == '-0':
                        s = '0'
                row.append(s)
            rows.append(row)
        save(raw / 'oracle.json', {'rows': sorted({tuple(row) for row in rows}), 'decimal_columns': decimals})
        print('PREPARED', name, len(rows), flush=True)
    save(OUT / (artifact_name('environment') + '.json'), {
        'sf': SF, 'duckdb_version': duckdb.__version__, 'database': database,
        'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
        'machine': os.uname()._asdict() if hasattr(os.uname(), '_asdict') else list(os.uname()),
        'affinity': sorted(os.sched_getaffinity(0)), 'runs': RUNS, 'cap_seconds': CAP_SECONDS, 'query_names': NAMES,
        **({'configuration': 'par', 'rayon_num_threads': THREADS, 'plain_only': True,
            'verification_only': SF == 0.1, 'runs': 0 if SF == 0.1 else RUNS} if PAR else {}),
        **({'compiler_runtime_sha256': compiler_fingerprint(),
            'git_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()} if PAR else {}),
        'source_sha256': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in [SOURCE, WORK / 'src/tpch_runtime.rs', WORK / 'xtask/src/tpch/worker.rs'] + [WORK / 'src/queries/retail/tpch' / (n + '.rs') for n in NAMES]},
    })


def adaptive():
    """Only alter generated measurement controls; preserve query and planner sources."""
    source = (WORK / 'xtask/src/tpch/worker.rs').read_text()
    if PAR:
        worker = CACHE_ROOT / 'adaptive-worker.rs'
        worker.parent.mkdir(parents=True, exist_ok=True)
        worker.write_text(source)
        save(OUT / (artifact_name('adaptive') + '.json'), {
            'generated_worker_sha256': hashlib.sha256(source.encode()).hexdigest(),
            'policy': 'One exact untimed execution at SF0.1; one exact warmup and five exact plain samples at SF1/SF10; 100-second process wall cap per query.',
            'changes': 'None. Unmodified authored worker, without counters or profile features.',
        })
        return
    source = source.replace('    let warm = query::run(&data, &args[1]);',
                            '    let warm_started = Instant::now();\n    let warm = query::run(&data, &args[1]);\n    let runs = if runs > 0 && warm_started.elapsed().as_secs_f64() > 60.0 { 3 } else { runs };')
    source = source.replace('    times.sort_by(f64::total_cmp);',
                            '    let runs = times.len();\n    times.sort_by(f64::total_cmp);')
    source = source.replace('    for _ in 0..runs {\n        let start = Instant::now();',
                            '    for _ in 0..runs {\n        if times.len() >= 3 && times.iter().any(|&ms| ms > 60000.0) { break; }\n        let start = Instant::now();')
    source = source.replace('    for _ in 0..runs {\n        counters::reset();',
                            '    for _ in 0..runs {\n        if samples.len() >= 3 && samples.iter().any(|s: &(f64, serde_json::Value)| s.0 > 60000.0) { break; }\n        counters::reset();')
    source = source.replace('    samples.sort_by(|a, b| a.0.total_cmp(&b.0));',
                            '    let runs = samples.len();\n    samples.sort_by(|a, b| a.0.total_cmp(&b.0));')
    source = source.replace('> 60.0', f'> {float(SLOW_SECONDS)}').replace('> 60000.0', f'> {float(SLOW_SECONDS * 1000)}')
    worker = CACHE_ROOT / 'adaptive-worker.rs'
    worker.parent.mkdir(parents=True, exist_ok=True)
    worker.write_text(source)
    save(OUT / (artifact_name('adaptive') + '.json'), {
        'generated_worker_sha256': hashlib.sha256(source.encode()).hexdigest(),
        'policy': f'{CAP_SECONDS}-second combined wall budget per configuration; warm query >{SLOW_SECONDS}s selects three timed runs, otherwise stop after three when a timed sample exceeds {SLOW_SECONDS}s. No plain-run pruning. Profile winners only: {PROFILE_WINNERS_ONLY}. Completed measurements are retained.',
        'changes': 'Only warm clock, sample count, loop stopping and effective median index in generated worker. Authored worker, query sources and compiler/plans unchanged.',
    })


def measure():
    for name in NAMES:
        raw = RAW / name
        crate = crate_directory(name)
        if PAR:
            stamp = build_stamp(name)
            validate_raw_stamp(name, stamp, required=True)
            if json.loads((crate / 'build_stamp.json').read_text()) != stamp:
                raise RuntimeError(f'{crate}: stale binary; run the build phase before measuring')

        def run_modes(config, modes):
            directory = raw / config
            directory.mkdir(parents=True, exist_ok=True)
            if (directory / ('plain.json' if PAR else 'profile.json')).exists() or (directory / 'status.json').exists():
                return
            budget_path = directory / 'budget.json'
            budget = json.loads(budget_path.read_text()) if budget_path.exists() else {
                'wall_seconds': 0, 'cap_seconds': CAP_SECONDS,
                'slow_seconds': SLOW_SECONDS, 'profile_winners_only': PROFILE_WINNERS_ONLY,
                'retained_plain_before_policy_change': (directory / 'plain.json').exists(),
                **({'configuration': 'par', 'rayon_num_threads': THREADS, 'plain_only': True,
                    'verification_only': SF == 0.1, 'runs': 0 if SF == 0.1 else RUNS} if PAR else {}),
            }
            for kind, binary, runs in modes:
                if (directory / (kind + '.json')).exists():
                    continue
                env = dict(ENV)
                if kind == 'profile':
                    env.update(CQ_OPERATOR_PROFILE='1', CQ_OPERATOR_COUNTS=str(directory / 'counts.json'))
                start = time.monotonic()
                try:
                    invoke([crate / binary, CACHE / 'data' / name, config, runs, directory / (kind + '.answers.json'), directory / (kind + '.json'), raw / 'oracle.json'], directory / 'commands.log', env, max(0.001, CAP_SECONDS - budget['wall_seconds']))
                except (subprocess.TimeoutExpired, subprocess.CalledProcessError) as error:
                    save(directory / 'status.json', {'status': 'timeout' if isinstance(error, subprocess.TimeoutExpired) else 'error', 'phase': kind, 'worker_wall_limit_seconds': CAP_SECONDS, 'configuration_wall_seconds': budget['wall_seconds'] + time.monotonic() - start, 'warm_correctness': 'ok' if (directory / (kind + '.answers.json')).exists() else 'unavailable', 'correctness': 'ok' if (directory / 'plain.json').exists() else 'unavailable', 'error': str(error)})
                    return
                finally:
                    budget['wall_seconds'] += time.monotonic() - start
                    save(budget_path, budget)
            print('MEASURED', SF, name, config, [m[0] for m in modes], flush=True)

        plain = [('plain', 'timing-worker', 0 if PAR and SF == 0.1 else RUNS)]
        if PAR:
            run_modes('par', plain)
            continue
        profile = [('counts', 'counting-worker', 0), ('profile', 'profile-worker', RUNS)]
        if not PROFILE_WINNERS_ONLY:
            for config in CONFIGS:
                run_modes(config, plain + profile)
            continue
        for config in CONFIGS:
            run_modes(config, plain)
        candidates = [(c, json.loads((raw / c / 'plain.json').read_text())['median_ms']) for c in ['le', 'le_runs'] if (raw / c / 'plain.json').exists()]
        winners = {'default', 'advised'}
        if candidates:
            winners.add(min(candidates, key=lambda pair: pair[1])[0])
        for config in CONFIGS:
            if config in winners:
                run_modes(config, profile)
            elif (raw / config / 'plain.json').exists() and not (raw / config / 'profile.json').exists() and not (raw / config / 'status.json').exists():
                save(raw / config / 'status.json', {'status': 'plain_only_not_selected', 'correctness': 'ok', 'reason': 'Profile omitted by user-authorized column-winner policy; all plain candidates measured.'})


def consolidate_par():
    rows = []
    for name in CATALOG_NAMES:
        if (RAW / name / 'par').exists():
            validate_raw_stamp(name, build_stamp(name), required=True)
        directory = RAW / name / 'par'
        path = directory / 'plain.json'
        plain = json.loads(path.read_text()) if path.exists() else None
        path = directory / 'status.json'
        status = json.loads(path.read_text()) if path.exists() else {
            'status': 'complete' if plain else 'missing',
        }
        if plain:
            assert not plain['instrumented']
            if SF == 0.1:
                assert plain['verified']
            else:
                assert len(plain['samples_ms']) == RUNS
        path = directory / 'budget.json'
        rows.append({
            'sf': SF, 'query': name, 'configuration': 'par', 'threads': THREADS,
            'status': status['status'], 'status_details': status,
            'correctness': 'ok' if plain else 'unavailable',
            'verification_only': SF == 0.1,
            'runs': len(plain.get('samples_ms', [])) if plain else 0,
            'answer_rows': plain['answer_rows'] if plain else None,
            'uninstrumented_total_median_ms': plain.get('median_ms') if plain else None,
            'uninstrumented_samples_ms': plain.get('samples_ms', []) if plain else [],
            'load_ms': plain.get('load_ms') if plain else None,
            'zone_build_ms': plain.get('zone_build_ms') if plain else None,
            'peak_rss_kib': plain.get('peak_rss_kib') if plain else None,
            'measurement_policy': json.loads(path.read_text()) if path.exists() else None,
            'raw_directory': str(directory.relative_to(ROOT)),
        })
    save(OUT / (artifact_name() + '.json'), {
        'schema_version': 1, 'sf': SF, 'configuration': 'par', 'threads': THREADS,
        'compiler_runtime_sha256': compiler_fingerprint(),
        'methodology': {
            'scope': 'All 27 catalog queries; fixed par configuration; no selection or instrumented modes.',
            'invocations': 'One untimed exact execution at SF0.1; one warmup plus five timed runs at SF1/SF10.',
            'cap_seconds': CAP_SECONDS, 'rayon_num_threads': THREADS,
            'correctness': 'Every warm and timed result matches the authored DuckDB SQL as a normalized exact set; DECIMAL trailing-zero normalization only.',
            'plain_boundary': 'query::run including query-local construction; excludes loading, canonicalization, comparison and final answer destruction.',
            'timeout': 'Process wall budget includes loading, warmup, timed runs and exact checks; never a numeric query timing.',
        },
        'rows': rows,
    })
    columns = [k for k in rows[0] if k not in ['status_details', 'measurement_policy', 'uninstrumented_samples_ms']]
    with (OUT / (artifact_name() + '.tsv')).open('w') as stream:
        writer = csv.DictWriter(stream, fieldnames=columns, delimiter='\t', extrasaction='ignore')
        writer.writeheader()
        writer.writerows(rows)
    print(json.dumps({'sf': SF, 'threads': THREADS, 'queries': len(rows),
                      'exact_complete': sum(r['correctness'] == 'ok' for r in rows)}))


def consolidate():
    if PAR:
        consolidate_par()
        return
    planner = {
        'default': 'written_coarse', 'no-reduce': 'written_coarse_no_reduce',
        'transfer': 'transfer', 'le': 'le', 'le_runs': 'le',
        'hypergraph': 'hypergraph', 'hypergraph_zone': 'hypergraph',
        'generic_smallest': 'written_fine', 'advised': 'written_coarse_advised',
    }
    rows = []
    for name in NAMES:
        raw = RAW / name
        for config in CONFIGS:
            directory = raw / config
            metrics = {kind: json.loads((directory / (kind + '.json')).read_text()) if (directory / (kind + '.json')).exists() else None for kind in ['plain', 'counts', 'profile']}
            plain, counts, profile = [metrics[k] for k in ['plain', 'counts', 'profile']]
            status = json.loads((directory / 'status.json').read_text()) if (directory / 'status.json').exists() else {'status': 'complete' if plain and profile else 'missing'}
            if plain:
                assert len(plain['samples_ms']) in [3, 5] and not plain['instrumented']
            if profile:
                assert len(profile['samples_ms']) in [3, 5] and counts['verified'] and profile['verified']
                assert counts['operator_rows'] == profile['operator_rows']
                assert counts['rows_copied'] == profile['bag_insertions']
            build = float(re.search(r'BUILD_EXCLUSIVE_TOTAL t=([0-9.]+)ms', profile['tree'])[1]) if profile else None
            rows.append({
                'sf': SF, 'query': name, 'configuration': config,
                'planner': planner[config], 'classification': 'default' if config == 'default' else ('le-kernel' if planner[config] == 'le' else 'other'),
                'status': status['status'], 'status_details': status,
                'measurement_policy': json.loads((directory / 'budget.json').read_text()) if (directory / 'budget.json').exists() else {'cap_seconds': 600, 'slow_seconds': 60, 'profile_winners_only': False},
                'runs': len(plain['samples_ms']) if plain else 0,
                'profile_runs': len(profile['samples_ms']) if profile else 0,
                'correctness': 'ok' if plain else status.get('correctness', 'unavailable'),
                'answer_rows': plain['answer_rows'] if plain else None,
                'load_ms': plain['load_ms'] if plain else None,
                'zone_build_ms': plain['zone_build_ms'] if plain else None,
                'uninstrumented_total_median_ms': plain['median_ms'] if plain else None,
                'uninstrumented_samples_ms': plain['samples_ms'] if plain else [],
                'profile_total_median_ms': profile['median_ms'] if profile else None,
                'profile_build_at_median_total_ms': build,
                'profile_query_residual_at_median_total_ms': profile['median_ms'] - build if profile else None,
                'profile_samples_ms': profile['samples_ms'] if profile else [],
                'profile_overhead_percent': 100 * (profile['median_ms'] / plain['median_ms'] - 1) if profile and plain else None,
                'build_scope_kinds': sorted({op['kind'] for op in profile['operators'] if op['kind'].endswith('_BUILD')}) if profile else [],
                'profile_operator_graph_verified': True if profile else None,
                'profile_top_level_clock_verified': profile['top_level_clock_verified'] if profile else None,
                'primitive_build_counts_verified': counts['distinct_insertions'] == profile['set_insertions'] + counts['candidate_insertions'] if profile else None,
                'primitive_distinct_insertions': counts['distinct_insertions'] if counts else None,
                'operator_set_insertions': profile['set_insertions'] if profile else None,
                'primitive_candidate_insertions': counts['candidate_insertions'] if counts else None,
                'raw_directory': str(directory.relative_to(ROOT)),
            })
    selections = []
    for name in NAMES:
        group = [r for r in rows if r['query'] == name]
        le = [r for r in group if r['classification'] == 'le-kernel' and r['uninstrumented_total_median_ms'] is not None]
        selections.append({
            'sf': SF, 'query': name,
            'default': next(r for r in group if r['configuration'] == 'default'),
            'advised': next(r for r in group if r['configuration'] == 'advised'),
            'best_le': min(le, key=lambda r: r['uninstrumented_total_median_ms']) if le else None,
            'le_search_complete': all(r['uninstrumented_total_median_ms'] is not None for r in group if r['classification'] == 'le-kernel'),
            'best_le_unavailable_reason': None if le else ('Authored catalog exposes only default; no LE-kernel configuration.' if not any(r['classification'] == 'le-kernel' for r in group) else 'No completed LE-kernel timing under the bounded policy.'),
        })
    methodology = {
        'scope': f'All 27 catalog queries at SF{SF}; default, le, le_runs and advised, without pruning.',
        'invocations': f'One warm execution, {RUNS} plain runs, independent counters and {RUNS} profile runs. Latest policy: three runs above {SLOW_SECONDS} seconds, {CAP_SECONDS}-second combined wall cap, profile column winners only={PROFILE_WINNERS_ONLY}. Completed original 600-second/60-second-threshold measurements retained. Per-row measurement_policy and status_details retain actual policy and timeout cap.',
        'correctness': f'Exact normalized answer sets against identical authored SQL in /data/tpch-gpu/sf{SF}/dbgen.duckdb, checked before and after every timed execution; DECIMAL trailing-zero normalization only.',
        'plain_boundary': 'Instant surrounds query::run, including query-local preprocessing, index construction, collection/cloning into native answers. Parquet load, canonicalization, sorting, oracle comparison, serialization and final answer destruction are excluded.',
        'profile_boundary': 'Same query::run plus operator timing instrumentation. Report build and residual from the very same execution selected by median profile total; residual = profile total - exclusive build. These are NOT independent medians of phase samples.',
        'build_definition': 'Runtime render_operators BUILD_EXCLUSIVE_TOTAL sums elapsed.saturating_sub(source_elapsed) for every operator kind ending _BUILD. Source production is excluded from construction. The emitted tree rounds build total to 0.001ms; residual has corresponding rounding uncertainty. Inclusive stream times must not be summed.',
        'ranking': 'CPU-LE minimizes completed uninstrumented whole-query median among le and le_runs. Default and advised are separate. le_search_complete records whether both LE candidates completed.',
        'classification_evidence': ['features/workloads/src/tpch_runtime.rs:110-175,236-242,290-307', 'features/planners/plan/src/lib.rs:35-76', 'facade/mini-linq-macros/src/lib.rs:258-274'],
        'limitations': [
            'Profile-only insertion drains each build input into a Vec before construction; its buffering, clock and operator bookkeeping overhead can substantially alter execution. Profile phase numbers are instrumented, not an exact split of the uninstrumented median.',
            'No subtraction combines uninstrumented total and profile build. Both totals and overhead are retained.',
            'Load and any ZoneRows preparation are excluded from query total and separately reported; query-local preprocessing remains in total and residual unless covered by a build operator.',
            'Samples are sorted by the worker; execution order is not retained.',
            'Timeout covers process loading, warm verification, repeated measured executions and exact comparisons across modes; it is not a per-query timing.',
            'A completed plain measurement remains valid when a later mode times out; missing phase timings remain null.',
            'Primitive insertion-count agreement is recorded separately from exact answers and operator-graph checks.',
        ],
    }
    save(OUT / f'performance_cpu_sf{SF}.json', {'methodology': methodology, 'rows': rows, 'selections': selections})
    columns = [k for k in rows[0] if not k.endswith('_samples_ms') and k not in ['status_details', 'measurement_policy']]
    with (OUT / f'performance_cpu_sf{SF}.tsv').open('w') as stream:
        writer = csv.DictWriter(stream, fieldnames=columns, delimiter='\t', extrasaction='ignore')
        writer.writeheader()
        for row in rows:
            if row['correctness'] == 'ok':
                writer.writerow({**row, 'build_scope_kinds': ','.join(row['build_scope_kinds'])})
    with (OUT / f'performance_cpu_sf{SF}_status.tsv').open('w') as stream:
        writer = csv.DictWriter(stream, fieldnames=['query', 'configuration', 'classification', 'status', 'correctness', 'runs', 'profile_runs', 'raw_directory'], delimiter='\t', extrasaction='ignore')
        writer.writeheader()
        writer.writerows(rows)
    print(json.dumps({'queries': len(selections), 'configurations': len(rows), 'completed_plain': sum(r['correctness'] == 'ok' for r in rows), 'le_available': sum(s['best_le'] is not None for s in selections)}))


def cleanup():
    removed = []
    answer_paths = (path for name in NAMES for path in (RAW / name).rglob('*.answers.json')) if PAR else RAW.rglob('*.answers.json')
    for path in answer_paths:
        removed.append(str(path.relative_to(ROOT)))
        path.unlink()
    oracle_paths = [RAW / name / 'oracle.json' for name in NAMES] if PAR else RAW.glob('*/oracle.json')
    for path in oracle_paths:
        if not path.exists():
            continue
        removed.append(str(path.relative_to(ROOT)))
        path.unlink()
    save(OUT / (artifact_name('cleanup') + '.json'), {
        'removed_temporary_correctness_artifacts': removed,
        'reason': 'Exact comparisons completed inside workers; retain metrics, graphs, statuses and reproduction SQL, not large duplicate answer sets. prepare recreates oracles from the shared dbgen database.',
    })


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('phase', choices=['build', 'prepare', 'adaptive', 'measure', 'consolidate', 'cleanup', 'all'])
    parser.add_argument('--sf', choices=['0.1', '1', '10'], required=True)
    parser.add_argument('--runs', type=int, choices=[5], default=5)
    parser.add_argument('--cap-seconds', type=int, default=None)
    parser.add_argument('--slow-seconds', type=int, default=60)
    parser.add_argument('--profile-winners-only', action='store_true')
    parser.add_argument('--par', action='store_true', help='Fixed par config, plain-only, isolated artifacts; SF0.1 verifies without timing')
    parser.add_argument('--threads', type=int, choices=[1, 24], help='Required with --par; sets RAYON_NUM_THREADS explicitly')
    parser.add_argument('--query', action='append', choices=CATALOG_NAMES, help='Repeat to select a par smoke subset; consolidation still includes all catalog queries')
    parser.add_argument('--database', type=Path, help='Explicit DuckDB oracle/export database override; recorded in environment metadata')
    args = parser.parse_args()
    PAR, THREADS = args.par, args.threads
    DATABASE = args.database.resolve() if args.database is not None else None
    if args.query:
        if not PAR:
            parser.error('--query subsets require --par')
        NAMES = [name for name in CATALOG_NAMES if name in args.query]
    if PAR != (THREADS is not None):
        parser.error('--par and --threads must be supplied together')
    if args.sf == '0.1' and not PAR:
        parser.error('SF0.1 is the --par exactness phase')
    if PAR and args.profile_winners_only:
        parser.error('--par never collects instrumented profiles')
    if args.cap_seconds is None:
        args.cap_seconds = 100 if PAR else 600
    if args.cap_seconds <= 0 or (PAR and args.cap_seconds != 100):
        parser.error('--cap-seconds must be positive; the par protocol requires 100')
    SF = 0.1 if args.sf == '0.1' else int(args.sf)
    RUNS, CAP_SECONDS = args.runs, args.cap_seconds
    SLOW_SECONDS, PROFILE_WINNERS_ONLY = args.slow_seconds, args.profile_winners_only
    if PAR:
        CONFIGS = ['par']
        CACHE_ROOT = CACHE_ROOT / 'par'
        ENV['RAYON_NUM_THREADS'] = str(THREADS)
        for key in ['CQ_OPERATOR_PROFILE', 'CQ_OPERATOR_COUNTS']:
            ENV.pop(key, None)
    CACHE = CACHE_ROOT / f'sf{SF}'
    RAW = OUT / 'performance_cpu_raw'
    if PAR:
        RAW = RAW / 'par' / f't{THREADS}'
    RAW = RAW / f'sf{SF}'
    CACHE.mkdir(parents=True, exist_ok=True)
    # Parallel binaries/target are shared across scales and threads; data is scale-specific.
    ENV['CARGO_TARGET_DIR'] = str(CACHE_ROOT / 'target')
    if args.phase == 'all':
        for phase in [adaptive, build, prepare, measure, consolidate, cleanup]:
            phase()
    else:
        if args.phase == 'build':
            adaptive()
        globals()[args.phase]()
