#!/usr/bin/env python3
"""Reproduce SF1 CPU measurements without altering authored queries or compiler plans."""
import argparse
import csv
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
CACHE = Path('/tmp/mini-linq-performance-cpu')
WORK = ROOT / 'features/workloads'
SOURCE = ROOT / 'experiments/gpu/tpch/src/source.rs'
NAMES = ['retail_' + x for x in re.findall(r'"([a-z0-9_]+)"', SOURCE.read_text().split('sources!(', 1)[1].split(');', 1)[0])]
ENV = dict(os.environ, CARGO_BUILD_JOBS='1', CARGO_TARGET_DIR=str(CACHE / 'target'))


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + '\n')


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
    log = OUT / 'performance_cpu_build.log'
    for name in NAMES:
        crate = CACHE / 'crates' / name
        if (OUT / 'performance_cpu_raw' / name / 'description.json').exists() and all(
            (crate / binary).exists() for binary in ['timing-worker', 'counting-worker', 'profile-worker']
        ):
            continue
        (crate / 'src').mkdir(parents=True, exist_ok=True)
        manifest = f'[package]\nname = "tpch-derived-{name}"\nversion = "0.0.0"\nedition = "2024"\n[workspace]\n[features]\ncounters = ["cq-storage/counters"]\nprofile = ["cq-storage/operator-timing"]\n[dependencies]\n'
        for dep, path in dependencies.items():
            features = ', features = ["parquet"]' if dep == 'cq-load' else ''
            manifest += f'{dep} = {{ path = "{ROOT / path}"{features} }}\n'
        manifest += 'serde_json = "1"\n'
        (crate / 'Cargo.toml').write_text(manifest)
        main = '#![recursion_limit = "512"]\n'
        main += f'#[allow(unused_imports, unused_macros, dead_code)] #[path = "{WORK / "src/tpch_runtime.rs"}"] mod tpch_runtime;\n'
        main += f'#[path = "{WORK / "src/queries/retail/tpch" / (name + ".rs")}"] mod query;\n'
        main += f'#[path = "{WORK / "xtask/src/tpch/comparison.rs"}"] mod comparison;\n'
        main += f'include!("{WORK / "xtask/src/tpch/worker.rs"}");\n'
        (crate / 'src/main.rs').write_text(main)
        shutil.copy(WORK / 'Cargo.lock', crate / 'Cargo.lock')
        invoke(['cargo', 'metadata', '--offline', '--format-version', '1', '--manifest-path', crate / 'Cargo.toml'], log)
        for feature, binary in [(None, 'timing-worker'), ('counters', 'counting-worker'), ('profile', 'profile-worker')]:
            args = ['cargo', 'build', '--release', '--locked', '--offline', '--manifest-path', crate / 'Cargo.toml']
            if feature:
                args += ['--features', feature]
            invoke(args, log)
            shutil.copy(CACHE / 'target/release' / ('tpch-derived-' + name), crate / binary)
        description = json.loads(subprocess.check_output([crate / 'timing-worker', 'describe']))
        save(OUT / 'performance_cpu_raw' / name / 'description.json', description)
        print('BUILT', name, description['configs'], flush=True)


def prepare():
    import duckdb
    db = duckdb.connect('/data/tpch-gpu/sf1/dbgen.duckdb', read_only=True)
    db.execute('SET threads=1')
    for name in NAMES:
        raw = OUT / 'performance_cpu_raw' / name
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
    save(OUT / 'performance_cpu_environment.json', {
        'duckdb_version': duckdb.__version__, 'database': '/data/tpch-gpu/sf1/dbgen.duckdb',
        'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
        'machine': os.uname()._asdict() if hasattr(os.uname(), '_asdict') else list(os.uname()),
        'affinity': sorted(os.sched_getaffinity(0)), 'runs': 5, 'query_names': NAMES,
        'source_sha256': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in [SOURCE, WORK / 'src/tpch_runtime.rs', WORK / 'xtask/src/tpch/worker.rs'] + [WORK / 'src/queries/retail/tpch' / (n + '.rs') for n in NAMES]},
    })


def adaptive():
    """Only alter generated measurement controls; preserve query and planner sources."""
    source = (WORK / 'xtask/src/tpch/worker.rs').read_text()
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
    worker = CACHE / 'adaptive-worker.rs'
    worker.write_text(source)
    save(OUT / 'performance_cpu_adaptive.json', {
        'generated_worker_sha256': hashlib.sha256(source.encode()).hexdigest(),
        'policy': 'After priority override: 600-second combined wall budget per configuration. Warm query >60s selects 3 timed runs; otherwise first three timed samples >60s stops at three. Existing completed five-run workers retained.',
        'changes': 'Only warm clock, sample count, loop stopping and effective median index in generated worker. Original worker.rs, query sources and compiler/plans unchanged.',
    })
    for name in NAMES:
        raw = OUT / 'performance_cpu_raw' / name
        desc = json.loads((raw / 'description.json').read_text())
        if all((raw / c / 'profile.json').exists() for c in desc['configs']):
            continue
        crate = CACHE / 'crates' / name
        main = crate / 'src/main.rs'
        main.write_text(main.read_text().replace(str(WORK / 'xtask/src/tpch/worker.rs'), str(worker)))
        for feature, binary in [(None, 'timing-worker'), ('counters', 'counting-worker'), ('profile', 'profile-worker')]:
            args = ['cargo', 'build', '--release', '--locked', '--offline', '--manifest-path', crate / 'Cargo.toml']
            if feature:
                args += ['--features', feature]
            invoke(args, OUT / 'performance_cpu_build.log')
            shutil.copy(CACHE / 'target/release' / ('tpch-derived-' + name), crate / binary)
        print('ADAPTIVE', name, flush=True)


def measure():
    gpu = {r['query']: r['total_ms'] for r in json.loads((OUT / 'performance_gpu_summary.json').read_text())['results'] if r['sf'] == 1 and r.get('exact')}
    cheaper = {'le_runs': 'le', 'no-reduce': 'default', 'advised': 'default', 'hypergraph_zone': 'hypergraph'}
    tasks = [(name, c) for name in NAMES for c in json.loads((OUT / 'performance_cpu_raw' / name / 'description.json').read_text())['configs']]
    tasks.sort(key=lambda pair: ['default', 'le', 'le_runs'].index(pair[1]) if pair[1] in ['default', 'le', 'le_runs'] else 3)
    for name, config in tasks:
        raw = OUT / 'performance_cpu_raw' / name
        crate = CACHE / 'crates' / name
        directory = raw / config
        directory.mkdir(parents=True, exist_ok=True)
        if (directory / 'profile.json').exists() or (directory / 'status.json').exists():
            continue
        prior = raw / cheaper.get(config, '__none__') / 'plain.json'
        if prior.exists() and name.removeprefix('retail_') in gpu:
            prior_ms = json.loads(prior.read_text())['median_ms']
            if prior_ms > 10 * gpu[name.removeprefix('retail_')]:
                save(directory / 'status.json', {'status': 'skipped', 'correctness': 'not_run', 'reason': 'Cheaper same-planner-family configuration already >10x GPU', 'prior_configuration': cheaper[config], 'prior_total_ms': prior_ms, 'gpu_total_ms': gpu[name.removeprefix('retail_')]})
                continue
        start = time.monotonic()
        for kind, binary, runs in [('plain', 'timing-worker', 5), ('counts', 'counting-worker', 0), ('profile', 'profile-worker', 5)]:
            if (directory / (kind + '.json')).exists():
                continue
            env = dict(ENV)
            if kind == 'profile':
                env.update(CQ_OPERATOR_PROFILE='1', CQ_OPERATOR_COUNTS=str(directory / 'counts.json'))
            try:
                invoke([crate / binary, CACHE / 'data' / name, config, runs, directory / (kind + '.answers.json'), directory / (kind + '.json'), raw / 'oracle.json'], directory / 'commands.log', env, max(0.001, 600 - (time.monotonic() - start)))
            except (subprocess.TimeoutExpired, subprocess.CalledProcessError) as error:
                save(directory / 'status.json', {'status': 'timeout' if isinstance(error, subprocess.TimeoutExpired) else 'error', 'phase': kind, 'worker_wall_limit_seconds': 600, 'configuration_wall_seconds': time.monotonic() - start, 'warm_correctness': 'ok' if (directory / (kind + '.answers.json')).exists() else 'unavailable', 'correctness': 'ok' if (directory / 'plain.json').exists() else 'unavailable', 'per_invocation_lower_bound_ms': None, 'error': str(error)})
                break
        print('ATTEMPTED', name, config, flush=True)


def consolidate():
    planner = {
        'default': 'written_coarse', 'no-reduce': 'written_coarse_no_reduce',
        'transfer': 'transfer', 'le': 'le', 'le_runs': 'le',
        'hypergraph': 'hypergraph', 'hypergraph_zone': 'hypergraph',
        'generic_smallest': 'written_fine', 'advised': 'written_coarse_advised',
    }
    rows = []
    for name in NAMES:
        raw = OUT / 'performance_cpu_raw' / name
        desc = json.loads((raw / 'description.json').read_text())
        for config in desc['configs']:
            directory = raw / config
            metrics = {kind: json.loads((directory / (kind + '.json')).read_text()) if (directory / (kind + '.json')).exists() else None for kind in ['plain', 'counts', 'profile']}
            plain, counts, profile = [metrics[k] for k in ['plain', 'counts', 'profile']]
            status = json.loads((directory / 'status.json').read_text()) if (directory / 'status.json').exists() else {'status': 'complete'}
            if plain:
                assert len(plain['samples_ms']) in [3, 5] and not plain['instrumented']
            if profile:
                assert len(profile['samples_ms']) in [3, 5] and counts['verified'] and profile['verified']
                assert counts['operator_rows'] == profile['operator_rows']
                assert counts['rows_copied'] == profile['bag_insertions']
            build = float(re.search(r'BUILD_EXCLUSIVE_TOTAL t=([0-9.]+)ms', profile['tree'])[1]) if profile else None
            rows.append({
                'sf': 1, 'query': name, 'configuration': config,
                'planner': planner[config], 'classification': 'default' if config == 'default' else ('le-kernel' if planner[config] == 'le' else 'other'),
                'status': status['status'], 'status_details': status,
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
            'sf': 1, 'query': name,
            'default': next(r for r in group if r['configuration'] == 'default'),
            'best_le': min(le, key=lambda r: r['uninstrumented_total_median_ms']) if le else None,
            'le_search_complete': all(r['uninstrumented_total_median_ms'] is not None for r in group if r['classification'] == 'le-kernel'),
            'best_le_unavailable_reason': None if le else ('Authored catalog exposes only default; no LE-kernel configuration.' if not any(r['classification'] == 'le-kernel' for r in group) else 'No completed LE-kernel timing under the bounded policy.'),
        })
    methodology = {
        'scope': 'All 27 names in GPU source.rs SOURCES, SF1, all configurations inventoried including no-reduce. Parent priority override authorizes 600-second combined wall budget/config, adaptive three-run timing, and same-family >10x GPU pruning.',
        'invocations': 'Fresh release workers include unchanged authored queries and comparison/runtime. First phase used stock worker: one warm execution then five timed runs. After priority override generated harness uses three timed runs when warm query or any first-three timed run exceeds60s, otherwise five. Independent counters warm run and separately instrumented profile worker. Actual run counts recorded per row.',
        'correctness': 'DuckDB Python executes identical authored SQL over the same /data/tpch-gpu/sf1/dbgen.duckdb. Compare normalized exact answer sets before measurement and after every timed run. Only DECIMAL columns have trailing zeros normalized; no floating tolerance. Separate counter/profile operator row graphs and primitive insertion totals are also checked.',
        'plain_boundary': 'Instant surrounds query::run, including query-local preprocessing, index construction, collection/cloning into native answers. Parquet load, canonicalization, sorting, oracle comparison, serialization and final answer destruction are excluded.',
        'profile_boundary': 'Same query::run plus operator timing instrumentation. Report build and residual from the very same execution selected by median profile total; residual = profile total - exclusive build. These are NOT independent medians of phase samples.',
        'build_definition': 'Runtime render_operators BUILD_EXCLUSIVE_TOTAL sums elapsed.saturating_sub(source_elapsed) for every operator kind ending _BUILD. Source production is excluded from construction. The emitted tree rounds build total to 0.001ms; residual has corresponding rounding uncertainty. Inclusive stream times must not be summed.',
        'ranking': 'Best observed LE minimizes uninstrumented whole-query median among completed cq_planners::order(le) configurations (le and le_runs). Family pruning means it is not necessarily the global best; le_search_complete flags coverage. Default is separate and never substitutes for unavailable LE.',
        'classification_evidence': ['features/workloads/src/tpch_runtime.rs:110-175,236-242,290-307', 'features/planners/plan/src/lib.rs:35-76', 'facade/mini-linq-macros/src/lib.rs:258-274'],
        'limitations': [
            'Profile-only insertion drains each build input into a Vec before construction; its buffering, clock and operator bookkeeping overhead can substantially alter execution. Profile phase numbers are instrumented, not an exact split of the uninstrumented median.',
            'No subtraction combines uninstrumented total and profile build. Both totals and overhead are retained.',
            'Load and any ZoneRows preparation are excluded from query total and separately reported; query-local preprocessing remains in total and residual unless covered by a build operator.',
            'Samples are sorted by the worker; execution order is not retained. First phase completed five-run results are not discarded or retrospectively censored.',
            'Only SF1 is collected here; no CPU SF10 claim.',
            'Timeout is a wall limit across process load, warm verification, measured runs, correctness checks and separate modes; it is NOT a lower bound on one query invocation.',
            'Rows with successful plain timing but missing/timed-out profile retain uninstrumented total and correctness; phase split remains null.',
            'The 12 standalone default-only queries pass exact-answer and operator-graph checks, but their primitive distinct_insertions counters disagree with operator set_insertions. primitive_build_counts_verified=false records this diagnostic separately; no independent set-insertion verification is claimed for these rows.',
        ],
    }
    save(OUT / 'performance_cpu_sf1.json', {'methodology': methodology, 'rows': rows, 'selections': selections})
    columns = [k for k in rows[0] if not k.endswith('_samples_ms') and k != 'status_details']
    with (OUT / 'performance_cpu_sf1.tsv').open('w') as stream:
        writer = csv.DictWriter(stream, fieldnames=columns, delimiter='\t', extrasaction='ignore')
        writer.writeheader()
        for row in rows:
            if row['correctness'] == 'ok':
                writer.writerow({**row, 'build_scope_kinds': ','.join(row['build_scope_kinds'])})
    with (OUT / 'performance_cpu_sf1_status.tsv').open('w') as stream:
        writer = csv.DictWriter(stream, fieldnames=['query', 'configuration', 'classification', 'status', 'correctness', 'runs', 'profile_runs', 'raw_directory'], delimiter='\t', extrasaction='ignore')
        writer.writeheader()
        writer.writerows(rows)
    print(json.dumps({'queries': len(selections), 'configurations': len(rows), 'completed_plain': sum(r['correctness'] == 'ok' for r in rows), 'le_available': sum(s['best_le'] is not None for s in selections)}))


def cleanup():
    removed = []
    for path in (OUT / 'performance_cpu_raw').rglob('*.answers.json'):
        removed.append(str(path.relative_to(ROOT)))
        path.unlink()
    for path in (OUT / 'performance_cpu_raw').glob('*/oracle.json'):
        removed.append(str(path.relative_to(ROOT)))
        path.unlink()
    save(OUT / 'performance_cpu_cleanup.json', {
        'removed_temporary_correctness_artifacts': removed,
        'reason': 'Exact comparisons completed inside workers; retain metrics, graphs, statuses and reproduction SQL, not large duplicate answer sets. prepare recreates oracles from the shared dbgen database.',
    })


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('phase', choices=['build', 'prepare', 'adaptive', 'measure', 'consolidate', 'cleanup'])
    args = parser.parse_args()
    globals()[args.phase]()
