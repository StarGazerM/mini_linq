#!/usr/bin/env python3
"""H05 GPU Datalog benchmark. Run with srdatalog-python's uv environment.

One child process per plan isolates native failures. Each child compiles once,
then initializes/loads/prepares a fresh DB for one warmup and five samples.
GPU memory is whole-device usage sampled at 1 Hz (a peak lower bound).
TSV integer values fit the engine's native int domain at SF1 and SF10.
"""
from __future__ import annotations
import argparse
import ctypes as C
import json
import os
from pathlib import Path
import statistics
import subprocess
import sys
import threading
import time
import traceback

HERE = Path(__file__).resolve().parent
ORDERS = {'fact': ['o', 's', 'c', 'd', 'n', 'r'],
          'dim': ['r', 'n', 'c', 'o', 'd', 's'], 'default': None}
RELATIONS = [('Customer', 2), ('Orders', 3), ('Lineitem', 2),
             ('Supplier', 2), ('Nation', 2), ('Region', 2)]


def build_program(constants, order, clause_order=None, semi_join=False, dimension_filters=False):
    from srdatalog.dsl import Var, Relation, Program, Filter
    c, n, o, d, s, r = map(Var, ('c', 'n', 'o', 'd', 's', 'r'))
    Customer, Orders, Lineitem, Supplier, Nation, Region = [
        Relation(name, arity, input_file=name.lower() + '.tsv')
        for name, arity in RELATIONS]
    Answer = Relation('Answer', 3, print_size=True)
    date_filter = Filter(('d',), f"return d >= {constants['date_start_days']} && d < {constants['date_end_days']};")
    rules = []
    if dimension_filters:
        AsiaRegion = Relation('AsiaRegion', 1)
        AsiaNation = Relation('AsiaNation', 1)
        rules = [(AsiaRegion(r) <= Region(r, constants['asia_id'])).named('AsiaRegion'),
                 (AsiaNation(n) <= Nation(n, r) & AsiaRegion(r)).named('AsiaNation')]
        rule = (Answer(o, s, n) <= Lineitem(o, s) & Orders(o, c, d)
                & date_filter & Customer(c, n) & AsiaNation(n) & Supplier(s, n)).named('H05')
        order = [v for v in order if v != 'r'] if order is not None else None
    else:
        rule = (Answer(o, s, n) <= Lineitem(o, s) & Orders(o, c, d)
                & date_filter & Customer(c, n) & Nation(n, r)
                & Region(r, constants['asia_id']) & Supplier(s, n)).named('H05')
    if order is not None or clause_order is not None:
        rule = rule.with_plan(var_order=order, clause_order=clause_order)
    if semi_join:
        rule = rule.with_semi_join()
    return Program(rules=rules + [rule])


def variant_label(order, explicit, clause_order, semi_join, dimension_filters):
    label = ''.join(explicit) if explicit else order
    if clause_order is not None:
        label += '_clauses' + ''.join(map(str, clause_order))
    if semi_join:
        label += '_semi'
    if dimension_filters:
        label += '_filters'
    return label


def var_order_arg(value):
    order = value.split(',')
    if sorted(order) != sorted('oscdnr'):
        raise argparse.ArgumentTypeError('expected each of o,s,c,d,n,r exactly once')
    return order


def clause_order_arg(value):
    try:
        order = [int(v) for v in value.split(',')]
    except ValueError as error:
        raise argparse.ArgumentTypeError('expected comma-separated clause indices') from error
    if sorted(order) != list(range(len(order))):
        raise argparse.ArgumentTypeError('clause order must be a permutation of 0..N-1')
    return order


class MemorySampler:
    def __init__(self):
        self.peak = None
    def __enter__(self):
        self.process = subprocess.Popen(['nvidia-smi', '--query-gpu=memory.used',
            '--format=csv,noheader,nounits', '-i', '0', '-l', '1'],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        def consume():
            for line in self.process.stdout:
                try:
                    value = int(line.strip())
                    self.peak = max(self.peak or 0, value)
                except ValueError:
                    pass
        self.thread = threading.Thread(target=consume, daemon=True)
        self.thread.start()
        return self
    def __exit__(self, *_):
        self.process.terminate()
        self.process.wait()
        self.thread.join()


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def child(args):
    from srdatalog import CompilerConfig, build_project, compile_jit_project
    from srdatalog.runtime import (runtime_include_paths, cuda_include_paths,
        runtime_defines, cuda_compile_flags, cuda_link_flags, cuda_libs)
    data = Path(args.data_root) / f'sf{args.sf[0]}'
    result_path = Path(args.child_result)
    selected = args.var_order[0] if args.var_order else ORDERS[args.order[0]]
    label = variant_label(args.order[0], args.var_order[0] if args.var_order else None,
                          args.clause_order, args.semi_join, args.dimension_filters)
    result = dict(sf=args.sf[0], order=label, compile_s=None, load_s=[],
        run_ms=[], median_ms=None, min_ms=None, answer_count=None,
        expected_count=int((data / 'answer.count').read_text()), exact=False,
        peak_vram_mib=None, peak_vram_note='whole GPU 0, 1 Hz sampled lower bound')
    save(result_path, result)
    try:
        manifest = json.loads((data / 'manifest.json').read_text())
        from srdatalog.ir.hir import compile_to_hir, compile_to_mir
        program = build_program(manifest['constants'], selected, args.clause_order,
                                args.semi_join, args.dimension_filters)
        hir = compile_to_hir(program)
        result['chosen_plans'] = [
            dict(rule=v.original_rule.name, var_order=v.var_order, clause_order=v.clause_order,
                 join_vars=sorted(v.join_vars), semijoin_patterns=repr(v.semijoin_patterns),
                 body=[repr(clause) for clause in v.original_rule.body])
            for stratum in hir.strata
            for v in stratum.base_variants + stratum.recursive_variants]
        result.update(requested_var_order=selected, requested_clause_order=args.clause_order,
                      semi_join=args.semi_join, dimension_filters=args.dimension_filters)
        print('CHOSEN_PLANS:', json.dumps(result['chosen_plans']), flush=True)
        mir_path = HERE / f'srdatalog_{label}.mir.txt'
        mir_path.write_text(repr(compile_to_mir(program, hir=hir)) + '\n')
        result['mir_file'] = str(mir_path)
        save(result_path, result)
        start = time.perf_counter()
        project = build_project(program, project_name='H05' + label,
                                cache_base=str(HERE / 'srdatalog_build'))
        result['project_dir'] = str(project['dir'])
        cfg = CompilerConfig(include_paths=runtime_include_paths() + cuda_include_paths(),
            defines=runtime_defines(), cxx_flags=cuda_compile_flags() + ['-fPIC'],
            link_flags=cuda_link_flags(), libs=cuda_libs() + ['boost_container'],
            shared=True, jobs=args.jobs)
        compiled = compile_jit_project(project, cfg)
        result['compile_s'] = time.perf_counter() - start
        if not compiled.ok():
            raise RuntimeError('\n'.join((x.stderr or x.stdout or '') for x in compiled.compile_results if x.returncode))
        result['artifact'] = str(compiled.artifact)
        print(f"compile_s={result['compile_s']:.6f}", flush=True)
        lib = C.CDLL(str(compiled.artifact), mode=C.RTLD_GLOBAL)
        signatures = {'init': [], 'shutdown': [], 'prepare': [], 'synchronize': [],
            'load_csv': [C.c_char_p, C.c_char_p], 'run': [C.c_ulonglong],
            'get_size': [C.c_char_p, C.POINTER(C.c_ulonglong)],
            'export_tsv': [C.c_char_p, C.c_char_p]}
        for name, types in signatures.items():
            fn = getattr(lib, 'srdatalog_' + name, None)
            if fn is None and name == 'export_tsv':
                continue
            fn.argtypes, fn.restype = types, C.c_int
        def call(name, *values):
            rc = getattr(lib, 'srdatalog_' + name)(*values)
            if rc:
                raise RuntimeError(f'srdatalog_{name} returned {rc}')
        with MemorySampler() as memory:
            try:
                for iteration in range(6):
                    result['phase'] = f'iteration {iteration}: init/load'
                    save(result_path, result)
                    call('init')
                    start = time.perf_counter()
                    for name, _ in RELATIONS:
                        call('load_csv', name.encode(), str(data / (name.lower() + '.tsv')).encode())
                    load_s = time.perf_counter() - start
                    result['phase'] = f'iteration {iteration}: prepare'
                    save(result_path, result)
                    call('prepare')
                    call('synchronize')
                    result['phase'] = f'iteration {iteration}: run'
                    save(result_path, result)
                    start = time.perf_counter()
                    call('run', 0)
                    call('synchronize')
                    run_ms = (time.perf_counter() - start) * 1000
                    count = C.c_ulonglong()
                    call('get_size', b'Answer', C.byref(count))
                    result['answer_count'] = count.value
                    if count.value != result['expected_count']:
                        raise RuntimeError(f"COUNT MISMATCH: Answer={count.value}, expected={result['expected_count']}")
                    if iteration == 0:
                        result['warmup_ms'] = run_ms
                        result['warmup_load_s'] = load_s
                        if args.sf[0] == 1 and hasattr(lib, 'srdatalog_export_tsv'):
                            export = HERE / f'srdatalog_sf1_{label}.answer.tsv'
                            call('export_tsv', b'Answer', str(export).encode())
                            actual = sorted(tuple(map(int, line.split())) for line in export.read_text().splitlines())
                            expected = [tuple(map(int, line.split())) for line in (data / 'answer.tsv').read_text().splitlines()]
                            result['tuple_exact'] = actual == expected
                            if actual != expected:
                                mismatch = next(((a, b) for a, b in zip(actual, expected) if a != b), (len(actual), len(expected)))
                                raise RuntimeError(f'TUPLE MISMATCH: first actual/expected={mismatch!r}')
                    else:
                        result['load_s'].append(load_s)
                        result['run_ms'].append(run_ms)
                    print(f'sf={args.sf[0]} order={label} iteration={iteration} load_s={load_s:.6f} run_ms={run_ms:.6f} Answer={count.value}', flush=True)
                    call('shutdown')
                    result['peak_vram_mib'] = memory.peak
                    save(result_path, result)
            finally:
                result['peak_vram_mib'] = memory.peak
        result.update(median_ms=statistics.median(result['run_ms']), min_ms=min(result['run_ms']),
                      exact=True, phase='complete')
    except Exception:
        result['error'] = traceback.format_exc()
        print(result['error'], flush=True)
    save(result_path, result)
    print(json.dumps(result), flush=True)
    return 0 if result['exact'] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sf', nargs='+', type=int, choices=[1, 10], default=[1, 10])
    parser.add_argument('--order', nargs='+', choices=list(ORDERS))
    parser.add_argument('--var-order', action='append', type=var_order_arg,
                        help='explicit comma-separated variable order; repeat for multiple variants')
    parser.add_argument('--clause-order', type=clause_order_arg,
                        help='clause indices after constant rewriting (8 clauses normally, 6 with dimension filters)')
    parser.add_argument('--semi-join', action='store_true', help='opt into the materialized semi-join rewrite')
    parser.add_argument('--dimension-filters', action='store_true',
                        help='derive static unary AsiaRegion/AsiaNation filters inside the timed query')
    parser.add_argument('--output', type=Path, default=HERE / 'srdatalog_h05.results.json')
    parser.add_argument('--append', action='store_true', help='append variants to an existing output')
    parser.add_argument('--data-root', default=os.environ.get('TPCH_DATA_ROOT', 'data/tpch-gpu'),
                        help='export parent containing sf1/ and sf10/, not an individual scale-factor directory')
    parser.add_argument('--jobs', type=int, default=8)
    parser.add_argument('--child-result')
    args = parser.parse_args()
    if args.order is None:
        args.order = [] if args.var_order else list(ORDERS)
    if args.clause_order is not None and len(args.clause_order) != (6 if args.dimension_filters else 8):
        parser.error('--clause-order needs 8 indices (6 with --dimension-filters), including rewritten constant filters')
    if args.child_result:
        return child(args)
    results = json.loads(args.output.read_text()) if args.append and args.output.exists() else []
    previous_count = len(results)
    output = args.output
    variants = ([(order, None) for order in args.order] +
                [(None, explicit) for explicit in (args.var_order or [])])
    for sf in args.sf:
        data = Path(args.data_root) / f'sf{sf}'
        deadline = time.monotonic() + (1800 if sf == 1 else 7200)
        required = ['manifest.json', 'answer.count', 'answer.tsv'] + [n.lower() + '.tsv' for n, _ in RELATIONS]
        while not all((data / name).exists() for name in required):
            if time.monotonic() >= deadline:
                raise TimeoutError(f'Timed out waiting for complete {data}')
            time.sleep(30)
        for order, explicit in variants:
            label = variant_label(order, explicit, args.clause_order, args.semi_join, args.dimension_filters)
            result_path = HERE / f'srdatalog_sf{sf}_{label}.json'
            cmd = [sys.executable, '-u', str(Path(__file__).resolve()), '--sf', str(sf),
                   '--order', order or 'default', '--data-root', args.data_root, '--jobs', str(args.jobs),
                   '--child-result', str(result_path)]
            if explicit:
                cmd += ['--var-order', ','.join(explicit)]
            if args.clause_order is not None:
                cmd += ['--clause-order', ','.join(map(str, args.clause_order))]
            if args.semi_join:
                cmd.append('--semi-join')
            if args.dimension_filters:
                cmd.append('--dimension-filters')
            print('COMMAND:', ' '.join(cmd), flush=True)
            log_path = HERE / f'srdatalog_sf{sf}_{label}.log'
            with log_path.open('w') as log:
                process = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
                for line in process.stdout:
                    print(line, end='', flush=True)
                    log.write(line)
                    log.flush()
                returncode = process.wait()
            result = json.loads(result_path.read_text()) if result_path.exists() else {'sf': sf, 'order': label, 'exact': False}
            result['returncode'] = returncode
            result['console_log'] = str(log_path)
            if returncode and 'error' not in result:
                result['error'] = f'Child terminated with returncode {returncode}; see {log_path}'
            results.append(result)
            save(output, results)
    return int(any(not result.get('exact') for result in results[previous_count:]))


if __name__ == '__main__':
    raise SystemExit(main())
