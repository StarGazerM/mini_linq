#!/usr/bin/env python3
"""Time DuckDB on H05 with the same scope as h05-reference.

Scope: base tables already resident in DuckDB's native compressed storage
(the dbgen database, attached read-only); the query computes the complete
SELECT DISTINCT answer and materializes it as a resident temp table. Timed
in-process with a host wall clock around that statement only: no parsing of
input files, no result transfer to the client, no CLI startup. One warmup is
excluded; the answer is checked tuple-for-tuple against answer.tsv.

Run: python3 experiments/gpu/compare/duckdb_h05_exact.py [--sf 1 10] [--threads 24 1]
"""
import argparse
import json
import os
from pathlib import Path
import statistics
import time

import duckdb

from export_tpch import QUERY


def measure(root, sf, threads, repeat):
    data = root / f'sf{sf}'
    expected = sorted(
        tuple(int(v) for v in line.split('\t')) for line in (data / 'answer.tsv').read_text().splitlines()
    )
    connection = duckdb.connect()
    connection.execute(f"ATTACH '{data / 'dbgen.duckdb'}' AS tpch (READ_ONLY)")
    connection.execute('USE tpch')
    connection.execute(f'SET threads={threads}')
    statement = f'CREATE OR REPLACE TEMP TABLE answer AS {QUERY}'
    samples = []
    for iteration in range(repeat + 1):
        start = time.perf_counter()
        connection.execute(statement)
        elapsed = (time.perf_counter() - start) * 1000
        if iteration == 0:
            actual = sorted(connection.execute('SELECT * FROM temp.answer').fetchall())
            if actual != expected:
                raise RuntimeError(f'SF{sf} threads={threads}: {len(actual)} rows vs {len(expected)} expected')
        else:
            samples.append(elapsed)
    connection.close()
    return {
        'sf': sf,
        'threads': threads,
        'duckdb_version': duckdb.__version__,
        'wall_ms': {'median': statistics.median(samples), 'min': min(samples), 'samples': samples},
        'answers': len(expected),
        'exact': True,
        'warmup_excluded': 1,
        'scope': 'resident native storage -> SELECT DISTINCT materialized as resident temp table',
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sf', nargs='+', type=int, default=[1, 10])
    parser.add_argument('--threads', nargs='+', type=int, default=[24, 1])
    parser.add_argument('--repeat', type=int, default=5)
    parser.add_argument('--root', type=Path, default=Path(os.environ.get('TPCH_DATA_ROOT', 'data/tpch-gpu')))
    parser.add_argument('--output', type=Path, default=Path('compare/duckdb_h05_exact.results.json'))
    arguments = parser.parse_args()
    results = [measure(arguments.root, sf, threads, arguments.repeat)
               for sf in arguments.sf for threads in arguments.threads]
    for result in results:
        print(f"SF{result['sf']} threads={result['threads']} median_ms={result['wall_ms']['median']:.3f} "
              f"min_ms={result['wall_ms']['min']:.3f} exact={result['exact']}")
    arguments.output.write_text(json.dumps(results, indent=1) + '\n')


if __name__ == '__main__':
    main()
