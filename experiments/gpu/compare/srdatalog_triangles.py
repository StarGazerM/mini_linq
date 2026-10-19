#!/usr/bin/env python3
"""Directed set-edge triangle references; never symmetrize the input.

Run with:
  uv run --project /home/stargazermiao/workspace/srdatalog-python python \
      experiments/gpu/compare/srdatalog_triangles.py

DuckDB CLI uses 24 threads, deduplicates edges, records graph statistics, and
counts E(a,b) JOIN E(b,c) JOIN E(a,c) once warm and five times measured.
The exported set edges feed the unmodified lsqb_triangle_count.py program,
including KnowsLoad and count-only TriangleJoin. Only TriangleJoin's plan is
replaced (default, a,b,c, b,a,c). Like srdatalog_h05.py, every sample initializes,
loads and prepares a fresh native database; synchronized srdatalog_run timing
includes KnowsLoad/index construction but excludes input load/prepare/H2D.
Native crashes are isolated per dataset/order and retain their last phase,
return code, console log, and any completed samples. Count agreement is not a
claim of tuple-by-tuple comparison. BFS demand is wedges * three uint32 values,
not a measured allocation, and excludes all auxiliary buffers.
"""
from __future__ import annotations

import argparse
import ctypes as C
import hashlib
import importlib.util
import json
import re
import signal
import statistics
import subprocess
import sys
import time
import traceback
from pathlib import Path

from srdatalog_h05 import MemorySampler, save

HERE = Path(__file__).resolve().parent
DATASETS = {
    'dblp': Path('/data/dataset/graph/com-dblp.ungraph.txt'),
    'livejournal': Path('/data/dataset/graph/soc-LiveJournal/edge.facts'),
}
ORDERS = {'default': None, 'abc': ['a', 'b', 'c'], 'bac': ['b', 'a', 'c']}
QUERY = ('SELECT count(*) AS triangle_count FROM E ab '
         'JOIN E bc ON ab.dst = bc.src '
         'JOIN E ac ON ac.src = ab.src AND ac.dst = bc.dst;')


def sql_string(value):
    return "'" + str(value).replace("'", "''") + "'"


def duckdb_reference(args, dataset):
    source = DATASETS[dataset]
    edges = args.data_root / f'{dataset}.set.tsv'
    sql_path = args.output.parent / f'triangles_{dataset}.duckdb.sql'
    log_path = args.output.parent / f'triangles_{dataset}.duckdb.log'
    sql = f""".bail on
SET threads=24;
CREATE TABLE raw_edges AS SELECT * FROM read_csv({sql_string(source)},
    delim='\\t', header=false, auto_detect=false, comment='#',
    columns={{'src':'INTEGER','dst':'INTEGER'}});
CREATE TABLE E AS SELECT DISTINCT src, dst FROM raw_edges;
SELECT 'stats' AS kind, (SELECT count(*) FROM raw_edges) AS input_rows,
    count(*) AS edge_count, (SELECT count(*) FROM raw_edges)-count(*) AS duplicates,
    least(min(src),min(dst)) AS node_min,
    greatest(max(src),max(dst)) AS node_max FROM E;
DROP TABLE raw_edges;
WITH indegrees AS (SELECT dst AS b, count(*)::HUGEINT AS n FROM E GROUP BY dst),
     outdegrees AS (SELECT src AS b, count(*)::HUGEINT AS n FROM E GROUP BY src)
SELECT 'wedges' AS kind, coalesce(sum(i.n*o.n),0)::HUGEINT AS wedges
FROM indegrees i JOIN outdegrees o USING(b);
COPY E TO {sql_string(edges)} (FORMAT CSV, HEADER false, DELIMITER '\\t');
.timer on
""" + '\n'.join([QUERY] * 6) + '\n.timer off\n'
    sql_path.write_text(sql)
    command = [str(args.duckdb), '-batch', '-json', ':memory:']
    start = time.perf_counter()
    completed = subprocess.run(command, input=sql, text=True,
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    elapsed = time.perf_counter() - start
    log_path.write_text(completed.stdout)
    if completed.returncode:
        raise RuntimeError(f'DuckDB exited {completed.returncode}; see {log_path}')
    rows = [row for line in completed.stdout.splitlines() if line.startswith('[')
            for row in json.loads(line)]
    stats = next(row for row in rows if row.get('kind') == 'stats')
    stats.pop('kind')
    wedges = int(next(row['wedges'] for row in rows if row.get('kind') == 'wedges'))
    counts = [int(row['triangle_count']) for row in rows if 'triangle_count' in row]
    times = [float(value) * 1000 for value in re.findall(
        r'Run Time \(s\): real ([0-9.]+)', completed.stdout)]
    if len(counts) != 6 or len(times) != 6 or len(set(counts)) != 1:
        raise RuntimeError(f'Incomplete/inconsistent DuckDB evidence; see {log_path}')
    return dict(dataset=dataset, source=str(source), source_bytes=source.stat().st_size,
                set_edges=str(edges), **stats, wedges=wedges,
                bfs_raw_triple_bytes=wedges * 12,
                bfs_raw_triple_gib=wedges * 12 / 2**30,
                duckdb=dict(command=command, threads=24, query=QUERY,
                            answer_count=counts[0], counts=counts,
                            warmup_ms=times[0], run_ms=times[1:],
                            median_ms=statistics.median(times[1:]), min_ms=min(times[1:]),
                            timer_resolution_ms=1, total_reference_s=elapsed,
                            sql_file=sql_path.name, console_log=log_path.name),
                srdatalog=[])


def build_program(checkout, order):
    """Load the actual upstream example and replace only its join plan."""
    example = checkout / 'examples/lsqb_triangle_count.py'
    spec = importlib.util.spec_from_file_location('triangle_count_example', example)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    program = module.build_triangle_db_program()
    for index, rule in enumerate(program.rules):
        if rule.name == 'TriangleJoin':
            rule = rule.with_plans([])
            if order is not None:
                rule = rule.with_plan(var_order=order)
            program.rules[index] = rule
    return program


def child(args):
    from srdatalog import CompilerConfig, build_project, compile_jit_project
    from srdatalog.ir.hir import compile_to_hir
    from srdatalog.runtime import (runtime_include_paths, cuda_include_paths,
        runtime_defines, cuda_compile_flags, cuda_link_flags, cuda_libs)

    dataset, order = args.dataset[0], args.order[0]
    result = dict(dataset=dataset, order=order, requested_var_order=ORDERS[order],
                  status='running', phase='compile', compile_s=None,
                  load_s=[], prepare_ms=[], run_ms=[], counts=[],
                  expected_count=args.expected_count, answer_count=None,
                  count_exact=False, peak_vram_mib=None,
                  peak_vram_note='whole GPU 0, 1 Hz sampled lower bound')
    save(args.child_result, result)
    try:
        program = build_program(args.srdatalog, ORDERS[order])
        hir = compile_to_hir(program)
        result['chosen_plans'] = [
            dict(rule=v.original_rule.name, var_order=v.var_order,
                 clause_order=v.clause_order, join_vars=sorted(v.join_vars),
                 body=[repr(clause) for clause in v.original_rule.body])
            for stratum in hir.strata
            for v in stratum.base_variants + stratum.recursive_variants]
        print('CHOSEN_PLANS:', json.dumps(result['chosen_plans']), flush=True)
        save(args.child_result, result)
        start = time.perf_counter()
        project = build_project(program, project_name='Triangles' + order,
                                cache_base=str(HERE / 'srdatalog_build'))
        result['project_dir'] = str(project['dir'])
        cfg = CompilerConfig(include_paths=runtime_include_paths() + cuda_include_paths(),
            defines=runtime_defines(), cxx_flags=cuda_compile_flags() + ['-fPIC'],
            link_flags=cuda_link_flags(), libs=cuda_libs() + ['boost_container'],
            shared=True, jobs=args.jobs)
        compiled = compile_jit_project(project, cfg)
        result['compile_s'] = time.perf_counter() - start
        if not compiled.ok():
            raise RuntimeError('\n'.join((x.stderr or x.stdout or '')
                               for x in compiled.compile_results if x.returncode))
        result['artifact'] = str(compiled.artifact)
        lib = C.CDLL(str(compiled.artifact), mode=C.RTLD_GLOBAL)
        signatures = {'init': [], 'shutdown': [], 'prepare': [], 'synchronize': [],
                      'load_csv': [C.c_char_p, C.c_char_p], 'run': [C.c_ulonglong],
                      'get_size': [C.c_char_p, C.POINTER(C.c_ulonglong)]}
        for name, types in signatures.items():
            fn = getattr(lib, 'srdatalog_' + name)
            fn.argtypes, fn.restype = types, C.c_int

        def call(name, *values):
            rc = getattr(lib, 'srdatalog_' + name)(*values)
            if rc:
                raise RuntimeError(f'srdatalog_{name} returned {rc}')

        with MemorySampler() as memory:
            for iteration in range(6):
                result['phase'] = f'iteration {iteration}: init/load'
                save(args.child_result, result)
                call('init')
                start = time.perf_counter()
                call('load_csv', b'KnowsInput',
                     str(args.data_root / f'{dataset}.set.tsv').encode())
                load_s = time.perf_counter() - start
                result['phase'] = f'iteration {iteration}: prepare'
                save(args.child_result, result)
                start = time.perf_counter()
                call('prepare')
                call('synchronize')
                prepare_ms = (time.perf_counter() - start) * 1000
                result['phase'] = f'iteration {iteration}: run'
                save(args.child_result, result)
                start = time.perf_counter()
                call('run', 0)
                call('synchronize')
                run_ms = (time.perf_counter() - start) * 1000
                count = C.c_ulonglong()
                call('get_size', b'Triangle', C.byref(count))
                result['answer_count'] = count.value
                result['counts'].append(count.value)
                if iteration == 0:
                    result.update(warmup_ms=run_ms, warmup_load_s=load_s,
                                  warmup_prepare_ms=prepare_ms)
                else:
                    result['load_s'].append(load_s)
                    result['prepare_ms'].append(prepare_ms)
                    result['run_ms'].append(run_ms)
                result['peak_vram_mib'] = memory.peak
                save(args.child_result, result)
                print(f'dataset={dataset} order={order} iteration={iteration} '
                      f'load_s={load_s:.6f} prepare_ms={prepare_ms:.6f} '
                      f'run_ms={run_ms:.6f} Triangle={count.value}', flush=True)
                if count.value != args.expected_count:
                    raise RuntimeError(f'COUNT MISMATCH: Triangle={count.value}, '
                                       f'expected={args.expected_count}')
                result['phase'] = f'iteration {iteration}: shutdown'
                save(args.child_result, result)
                call('shutdown')
            result['peak_vram_mib'] = memory.peak
        result.update(status='ok', phase='complete', count_exact=True,
                      median_ms=statistics.median(result['run_ms']),
                      min_ms=min(result['run_ms']))
    except Exception:
        result.update(status='error', error=traceback.format_exc())
        print(result['error'], flush=True)
    save(args.child_result, result)
    print(json.dumps(result), flush=True)
    return 0 if result['count_exact'] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--dataset', nargs='+', choices=list(DATASETS), default=list(DATASETS))
    parser.add_argument('--order', nargs='+', choices=list(ORDERS), default=list(ORDERS))
    parser.add_argument('--duckdb', type=Path,
                        default=Path('/home/stargazermiao/.duckdb/cli/latest/duckdb'))
    parser.add_argument('--srdatalog', type=Path,
                        default=Path('/home/stargazermiao/workspace/srdatalog-python'))
    parser.add_argument('--data-root', type=Path, default=Path('/tmp/triangles-reference-data'))
    parser.add_argument('--output', type=Path, default=HERE / 'triangles_references.results.json')
    parser.add_argument('--jobs', type=int, default=8)
    parser.add_argument('--child-result', type=Path)
    parser.add_argument('--expected-count', type=int)
    args = parser.parse_args()
    if args.child_result:
        return child(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.data_root.mkdir(parents=True, exist_ok=True)
    example = args.srdatalog / 'examples/lsqb_triangle_count.py'
    results = dict(query='Triangle(a,b,c) <= E(a,b) & E(b,c) & E(a,c)',
                   input_semantics='directed set edges; no symmetrization or distinct-node filter',
                   warmups=1, measured_runs=5,
                   timing_scope='srdatalog_run + synchronize; fresh DB each sample; includes '
                                'KnowsLoad/index builds; excludes load/prepare/H2D',
                   count_validation='count-only agreement, not exported tuple comparison',
                   duckdb_version=subprocess.check_output([str(args.duckdb), '--version'], text=True).strip(),
                   srdatalog_checkout=str(args.srdatalog),
                   srdatalog_revision=subprocess.check_output(
                       ['git', '-C', str(args.srdatalog), 'rev-parse', 'HEAD'], text=True).strip(),
                   example=str(example), example_sha256=hashlib.sha256(example.read_bytes()).hexdigest(),
                   datasets=[])
    save(args.output, results)
    for dataset in args.dataset:
        reference = duckdb_reference(args, dataset)
        results['datasets'].append(reference)
        save(args.output, results)
        print('DUCKDB:', json.dumps(reference), flush=True)
        for order in args.order:
            result_path = args.output.parent / f'triangles_{dataset}_{order}.json'
            log_path = args.output.parent / f'triangles_{dataset}_{order}.log'
            command = [sys.executable, '-u', str(Path(__file__).resolve()),
                       '--dataset', dataset, '--order', order, '--srdatalog', str(args.srdatalog),
                       '--data-root', str(args.data_root), '--jobs', str(args.jobs),
                       '--child-result', str(result_path),
                       '--expected-count', str(reference['duckdb']['answer_count'])]
            print('COMMAND:', ' '.join(command), flush=True)
            with log_path.open('w') as log:
                process = subprocess.Popen(command, stdout=subprocess.PIPE,
                                           stderr=subprocess.STDOUT, text=True)
                for line in process.stdout:
                    print(line, end='', flush=True)
                    log.write(line)
                    log.flush()
                returncode = process.wait()
            result = json.loads(result_path.read_text()) if result_path.exists() else dict(
                dataset=dataset, order=order, count_exact=False)
            result.update(returncode=returncode, command=command, console_log=log_path.name)
            if returncode:
                result['status'] = 'native_failure' if returncode < 0 else 'error'
                if returncode < 0:
                    result['signal'] = signal.Signals(-returncode).name
                result.setdefault('error', f'Child terminated with returncode {returncode}; see {log_path.name}')
            save(result_path, result)
            reference['srdatalog'].append(result)
            save(args.output, results)
        valid = [r for r in reference['srdatalog'] if r.get('count_exact') and r['returncode'] == 0]
        reference['best_order'] = min(valid, key=lambda r: r['median_ms'])['order'] if valid else None
        save(args.output, results)
    return int(any(not r.get('count_exact') for d in results['datasets'] for r in d['srdatalog']))


if __name__ == '__main__':
    raise SystemExit(main())
