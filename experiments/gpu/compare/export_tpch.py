#!/usr/bin/env python3
"""Export projected TPC-H Q5 inputs, exact answers, and DuckDB CPU timings.

Run: python3 experiments/gpu/compare/export_tpch.py [--sf 1 10]
Existing databases are reused; exports are replaced deterministically. SF1 is
copied from the original benchmark database, never modified in place. Timing
uses the complete SELECT DISTINCT, not a count wrapper, in one CLI process per
thread count with one warmup followed by five samples (DuckDB .timer wall time).
"""
import argparse
import itertools
import json
from pathlib import Path
import re
import shutil
import statistics
import subprocess

import numpy as np

CLI = '/home/stargazermiao/.duckdb/cli/latest/duckdb'
SOURCE_SF1 = Path('/data/mini_linq/features/workloads/target/bench/data/tpch-1/dbgen.duckdb')
RELATIONS = {
    'customer': [('c_custkey', 'i64'), ('c_nationkey', 'i32')],
    'orders': [('o_orderkey', 'i64'), ('o_custkey', 'i64'), ('o_orderdate', 'i32')],
    'lineitem': [('l_orderkey', 'i64'), ('l_suppkey', 'i64')],
    'supplier': [('s_suppkey', 'i64'), ('s_nationkey', 'i32')],
    'nation': [('n_nationkey', 'i32'), ('n_regionkey', 'i32')],
    'region': [('r_regionkey', 'i32'), ('r_name', 'i32')],
}
QUERY = """SELECT DISTINCT o.o_orderkey, s.s_suppkey, n.n_nationkey
FROM customer c
JOIN orders o ON c.c_custkey = o.o_custkey
JOIN lineitem l ON l.l_orderkey = o.o_orderkey
JOIN supplier s ON s.s_suppkey = l.l_suppkey AND s.s_nationkey = c.c_nationkey
JOIN nation n ON n.n_nationkey = s.s_nationkey
JOIN region r ON r.r_regionkey = n.n_regionkey
WHERE r.r_name = 'ASIA'
AND o.o_orderdate >= DATE '1994-01-01'
AND o.o_orderdate < DATE '1995-01-01'"""


def sql_string(value):
    return "'" + str(value).replace("'", "''") + "'"


def run_sql(database, sql):
    command = [CLI, '-batch', '-bail', '-csv', '-noheader', str(database)]
    result = subprocess.run(command, input=sql, text=True, capture_output=True, check=True)
    return result.stdout


def export(sf, base):
    root = base / f'sf{sf}'
    root.mkdir(parents=True, exist_ok=True)
    database = root / 'dbgen.duckdb'
    # A manifest is published last, so its presence means all exports completed.
    (root / 'manifest.json').unlink(missing_ok=True)
    if not database.exists():
        if sf == 1 and SOURCE_SF1.exists():
            temporary = root / 'dbgen.copying.duckdb'
            shutil.copy2(SOURCE_SF1, temporary)
            temporary.replace(database)
        else:
            temporary = root / 'dbgen.generating.duckdb'
            temporary.unlink(missing_ok=True)
            run_sql(temporary, f'SET threads=24; INSTALL tpch; LOAD tpch; CALL dbgen(sf={sf}); CHECKPOINT;\n')
            temporary.replace(database)
    dates = run_sql(database, "SELECT DATE '1994-01-01' - DATE '1970-01-01', DATE '1995-01-01' - DATE '1970-01-01';\n").strip()
    assert dates == '8766,9131', dates
    names = run_sql(database, 'SELECT DISTINCT r_name FROM region ORDER BY r_name;\n').splitlines()
    (root / 'region.r_name.dict.txt').write_text('\n'.join(names) + '\n')
    manifest = {'sf': sf, 'relations': {}, 'constants': {
        'date_start_days': 8766, 'date_end_days': 9131, 'asia_id': names.index('ASIA')}}
    for relation, columns in RELATIONS.items():
        expressions = []
        for column, kind in columns:
            if column == 'o_orderdate':
                expressions.append("(o_orderdate - DATE '1970-01-01')::INTEGER AS o_orderdate")
            elif column == 'r_name':
                cases = ' '.join(f'WHEN {sql_string(name)} THEN {index}' for index, name in enumerate(names))
                expressions.append(f'(CASE r_name {cases} END)::INTEGER AS r_name')
            else:
                expressions.append(column)
        tsv = root / f'{relation}.tsv'
        run_sql(database, f"SET threads=24; SET preserve_insertion_order=true; COPY (SELECT {', '.join(expressions)} FROM {relation}) TO {sql_string(tsv)} (FORMAT CSV, DELIMITER '\t', HEADER false);\n")
        expected = int(run_sql(database, f'SELECT count(*) FROM {relation};\n').strip())
        files = [root / f'{relation}.{column}.bin' for column, kind in columns]
        streams = [path.open('wb') for path in files]
        rows = 0
        try:
            with tsv.open() as source:
                while True:
                    lines = list(itertools.islice(source, 500_000))
                    if not lines:
                        break
                    values = np.loadtxt(lines, dtype=np.int64, delimiter='\t', ndmin=2)
                    rows += values.shape[0]
                    for index, (_, kind) in enumerate(columns):
                        values[:, index].astype('<i8' if kind == 'i64' else '<i4').tofile(streams[index])
        finally:
            for stream in streams:
                stream.close()
        assert rows == expected, (relation, rows, expected)
        for path, (_, kind) in zip(files, columns):
            assert path.stat().st_size == rows * (8 if kind == 'i64' else 4), path
        manifest['relations'][relation] = {'rows': rows, 'columns': [
            {'name': column, 'type': kind, 'file': path.name}
            for (column, kind), path in zip(columns, files)]}
        print(f'SF{sf} {relation}: {rows} rows; binary sizes verified', flush=True)
    answer = root / 'answer.tsv'
    run_sql(database, f"SET threads=24; COPY ({QUERY} ORDER BY 1,2,3) TO {sql_string(answer)} (FORMAT CSV, DELIMITER '\t', HEADER false);\n")
    with answer.open() as source:
        count = sum(1 for _ in source)
    (root / 'answer.count').write_text(f'{count}\n')
    timings = {'engine': 'DuckDB', 'version': subprocess.check_output([CLI, '--version'], text=True).strip(),
               'sf': sf, 'query': QUERY, 'units': 'seconds', 'warmup_runs': 1, 'measured_runs': 5,
               'method': 'CLI .timer real wall time for full SELECT DISTINCT; 1 ms resolution; one process per thread count',
               'threads': {}}
    for threads in (1, 24):
        output = run_sql(database, f'SET threads={threads};\n.timer on\n' + (QUERY + ';\n') * 6)
        (root / f'duckdb_timing_threads{threads}.log').write_text(output)
        samples = [float(value) for value in re.findall(r'Run Time \(s\): real ([0-9.]+)', output)]
        assert len(samples) == 6, samples
        timings['threads'][str(threads)] = {'warmup_seconds': samples[0], 'samples_seconds': samples[1:],
                                          'median_seconds': statistics.median(samples[1:])}
    (root / 'duckdb_timing.json').write_text(json.dumps(timings, indent=2) + '\n')
    (root / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    # Required handoff evidence: full directory metadata, manifest, count, timings.
    subprocess.run(['ls', '-la', str(root)], check=True)
    print((root / 'manifest.json').read_text(), flush=True)
    print(f'SF{sf} answer.count: {count}', flush=True)
    print((root / 'duckdb_timing.json').read_text(), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sf', nargs='+', type=int, choices=[1, 10], default=[1, 10])
    parser.add_argument('--root', type=Path, default=Path('/data/tpch-gpu'))
    arguments = parser.parse_args()
    for sf in sorted(set(arguments.sf)):
        export(sf, arguments.root)


if __name__ == '__main__':
    main()
