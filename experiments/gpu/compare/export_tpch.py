#!/usr/bin/env python3
"""Export all authored TPC-H CQ inputs, exact answers, and DuckDB CPU timings.

Run: python3 experiments/gpu/compare/export_tpch.py [--sf 1 10]
Existing databases are reused; exports are replaced deterministically. SF1 is
copied from the original benchmark database, never modified in place. Timing
uses the complete SELECT DISTINCT, not a count wrapper, in one CLI process per
thread count with one warmup followed by five samples (DuckDB .timer wall time).
"""
import argparse
import csv
import hashlib
import itertools
import io
import json
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess

import numpy as np

CLI = os.environ.get('DUCKDB', 'duckdb')
SOURCE_SF1 = Path(os.environ.get('TPCH_SOURCE_SF1',
    str(Path(__file__).resolve().parents[3] / 'features/workloads/target/bench/data/tpch-1/dbgen.duckdb')))
QUERY_DIR = Path(__file__).resolve().parents[3] / 'features/workloads/src/queries/retail/tpch'


def workloads():
    """Use the authored SQL and relation declarations as the export contract."""
    relations, queries = {}, {}
    for path in sorted(QUERY_DIR.glob('*.rs')):
        source = path.read_text()
        # Standalone aggregate kernels have a different export contract.
        if 'retail_kernel!' not in source:
            continue
        name = re.search(r'name "([^"]+)"', source).group(1)
        sql = re.search(r'sql r#"(.*?)"#;', source, re.S).group(1)
        required = {}
        for table, fields in re.findall(r'relation \w+ => (\w+)\(([^)]+)\);', source):
            required[table] = []
            for column, logical in re.findall(r'(\w+): (\w+)', fields):
                previous = relations.setdefault(table, {}).setdefault(column, logical)
                assert previous == logical, (table, column, previous, logical)
                required[table].append(column)
        assert required and sql.startswith('SELECT DISTINCT'), path
        queries[name] = {'sql': sql, 'source': str(path.relative_to(QUERY_DIR.parents[5])),
                         'required_columns': required,
                         'derived_relations': re.findall(
                             r'derived\s*\{\s*relation\s+(\w+)\(', source)}
    assert queries, QUERY_DIR
    return relations, queries


def checksum(path):
    digest = hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda: source.read(8 * 1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


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
    before = {path.name: checksum(path) for path in sorted(root.glob('*.bin'))}
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
    relations, queries = workloads()
    manifest = {'schema_version': 2, 'sf': sf, 'byte_order': 'little',
                'relations': {}, 'queries': queries, 'constants': {
                    'date_start_days': 8766, 'date_end_days': 9131}}
    for relation, columns in relations.items():
        expressions, metadata, joins = [], [], []
        for column, logical in columns.items():
            kind = 'i32' if logical in ('i32', 'Date', 'String') else 'i64'
            item = {'name': column, 'type': kind, 'logical_type': logical,
                    'encoding': 'plain', 'file': f'{relation}.{column}.bin'}
            expression = f't.{column}'
            if logical == 'Date':
                expression = f"(t.{column} - DATE '1970-01-01')::INTEGER"
                item.update(encoding='days_since_epoch', epoch='1970-01-01', unit='day')
            elif logical == 'Decimal':
                expression = f'(t.{column} * 100)::BIGINT'
                item.update(encoding='scaled_integer', scale=2)
            elif logical == 'String':
                # Comments/phone strings must be decoded from CSV, not retained
                # with CLI quoting (supplier comments can contain commas).
                names = [row[0] for row in csv.reader(io.StringIO(run_sql(
                    database, f"SELECT DISTINCT {column} FROM {relation} ORDER BY {column};\n")))]
                dictionary = root / f'{relation}.{column}.dict.txt'
                dictionary.write_text('\n'.join(names) + '\n')
                alias = f'd{len(joins)}'
                joins.append(f'JOIN (SELECT {column}, (row_number() OVER (ORDER BY {column}) - 1)::INTEGER AS code FROM (SELECT DISTINCT {column} FROM {relation})) {alias} ON t.{column} = {alias}.{column}')
                expression = f'{alias}.code'
                item.update(encoding='dictionary', dictionary=dictionary.name,
                            dictionary_entries=len(names), dictionary_index_base=0)
                if column == 'r_name':
                    manifest['constants']['asia_id'] = names.index('ASIA')
            expressions.append(f'{expression} AS {column}')
            metadata.append(item)
        tsv = root / f'{relation}.tsv'
        # rowid preserves original physical scan order even across dictionary joins.
        run_sql(database, f"SET threads=24; SET preserve_insertion_order=true; COPY (SELECT {', '.join(expressions)} FROM {relation} t {' '.join(joins)} ORDER BY t.rowid) TO {sql_string(tsv)} (FORMAT CSV, DELIMITER '\t', HEADER false);\n")
        expected = int(run_sql(database, f'SELECT count(*) FROM {relation};\n').strip())
        files = [root / item['file'] for item in metadata]
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
                    for index, item in enumerate(metadata):
                        values[:, index].astype('<i8' if item['type'] == 'i64' else '<i4').tofile(streams[index])
        finally:
            for stream in streams:
                stream.close()
        assert rows == expected, (relation, rows, expected)
        for path, item in zip(files, metadata):
            assert path.stat().st_size == rows * (8 if item['type'] == 'i64' else 4), path
        manifest['relations'][relation] = {'rows': rows, 'columns': metadata}
        tsv.unlink()
        print(f'SF{sf} {relation}: {rows} rows; binary sizes verified', flush=True)
    after = {name: checksum(root / name) for name in before}
    assert before == after, 'Existing binary column checksums changed'
    (root / 'binary_checksums.json').write_text(json.dumps(
        {'algorithm': 'sha256', 'before': before, 'after': after, 'identical': True}, indent=2) + '\n')
    for name, query in queries.items():
        sql = query['sql']
        answer = root / f'answer.{name}.tsv'
        run_sql(database, f"SET threads=24; COPY ({sql} ORDER BY ALL) TO {sql_string(answer)} (FORMAT CSV, DELIMITER '\t', HEADER false);\n")
        with answer.open() as source:
            count = sum(1 for _ in source)
        (root / f'answer.{name}.count').write_text(f'{count}\n')
        query.update(answer=answer.name, count=count)
        timings = {'engine': 'DuckDB', 'version': subprocess.check_output([CLI, '--version'], text=True).strip(),
                   'sf': sf, 'query': sql, 'units': 'seconds', 'warmup_runs': 1, 'measured_runs': 5,
                   'method': 'CLI .timer real wall time for full SELECT DISTINCT; 1 ms resolution; one process per thread count',
                   'threads': {}}
        for threads in (1, 24):
            log = root / f'duckdb_timing.{name}_threads{threads}.log'
            with log.open('w') as destination:
                subprocess.run([CLI, '-batch', '-bail', '-csv', '-noheader', str(database)],
                               input=f'SET threads={threads};\n.timer on\n' + (sql + ';\n') * 6,
                               text=True, stdout=destination, stderr=subprocess.PIPE, check=True)
            with log.open() as source:
                samples = [float(match.group(1)) for line in source
                           if (match := re.search(r'Run Time \(s\): real ([0-9.]+)', line))]
            assert len(samples) == 6, (name, threads, samples)
            timings['threads'][str(threads)] = {'warmup_seconds': samples[0], 'samples_seconds': samples[1:],
                                              'median_seconds': statistics.median(samples[1:])}
        timing_path = root / f'duckdb_timing.{name}.json'
        timing_path.write_text(json.dumps(timings, indent=2) + '\n')
        query['timing'] = timing_path.name
        if name == 'retail_h05':
            shutil.copyfile(answer, root / 'answer.tsv')
            shutil.copyfile(root / f'answer.{name}.count', root / 'answer.count')
            shutil.copyfile(timing_path, root / 'duckdb_timing.json')
            for threads in (1, 24):
                shutil.copyfile(root / f'duckdb_timing.{name}_threads{threads}.log',
                                root / f'duckdb_timing_threads{threads}.log')
        print(f"SF{sf} {name}: {count} answers; medians " +
              str({t: v['median_seconds'] for t, v in timings['threads'].items()}), flush=True)
    manifest['binary_bytes'] = sum(path.stat().st_size for path in root.glob('*.bin'))
    (root / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(f'SF{sf} binary bytes: {manifest["binary_bytes"]}', flush=True)


def main():
    global CLI, SOURCE_SF1
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sf', nargs='+', type=int, choices=[1, 10], default=[1, 10])
    parser.add_argument('--root', type=Path, default=Path(os.environ.get('TPCH_DATA_ROOT', 'data/tpch-gpu')),
                        help='export parent directory; writes sf1/ and sf10/ beneath it')
    parser.add_argument('--duckdb', default=CLI, help='DuckDB executable (default: DUCKDB or duckdb on PATH)')
    parser.add_argument('--source-sf1', type=Path, default=SOURCE_SF1,
                        help='optional existing SF1 database to copy; otherwise generate via DuckDB tpch')
    arguments = parser.parse_args()
    CLI, SOURCE_SF1 = arguments.duckdb, arguments.source_sf1
    for sf in sorted(set(arguments.sf)):
        export(sf, arguments.root)


if __name__ == '__main__':
    main()
