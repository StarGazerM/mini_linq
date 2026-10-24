#!/usr/bin/env python3
"""Measure authored SQL without CSV rendering; run only in an exclusive CPU window."""
import argparse
import datetime
import json
from pathlib import Path
import re
import statistics
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sf', nargs='+', type=int, choices=(1, 10), default=[1, 10])
    parser.add_argument('--duckdb', default='duckdb')
    args = parser.parse_args()
    output = Path(__file__).resolve().parent
    version = subprocess.check_output([args.duckdb, '--version'], text=True).strip()
    for sf in args.sf:
        prior = json.loads((output / f'performance_duck_sf{sf}.json').read_text())
        results = []
        for source in prior['results']:
            name = source['catalog_name']
            sql = source['raw_timing']['query']
            item = {key: source[key] for key in ('gpu_query', 'catalog_name', 'sql_source')}
            timing = {'engine': 'DuckDB', 'version': version, 'sf': sf, 'query': sql,
                      'units': 'seconds', 'warmup_runs': 1, 'measured_runs': 5,
                      'threads': {}}
            for threads in (1, 24):
                command = [args.duckdb, '-batch', '-bail', '-readonly',
                           f'/data/tpch-gpu/sf{sf}/dbgen.duckdb']
                script = f'SET threads={threads};\n.mode trash\n.timer on\n' + (sql + ';\n') * 6
                completed = subprocess.run(command, input=script, text=True,
                                           capture_output=True, check=True)
                log = completed.stdout + completed.stderr
                samples = [float(value) for value in
                           re.findall(r'Run Time \(s\): real ([0-9.]+)', log)]
                if len(samples) != 6:
                    raise RuntimeError(f'{name} SF{sf} t{threads}: expected six times: {log}')
                timing['threads'][str(threads)] = {
                    'warmup_seconds': samples[0], 'samples_seconds': samples[1:],
                    'median_seconds': statistics.median(samples[1:]), 'raw_cli_output': log}
            item['raw_timing'] = timing
            results.append(item)
            print(f'SF{sf} {name}: ' + str({t: v['median_seconds'] for t, v in timing['threads'].items()}), flush=True)
        result = {'schema_version': 1, 'sf': sf,
                  'captured_at_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  'methodology': {
                      'engine_version': version,
                      'timer': 'DuckDB CLI .timer real wall seconds, 1 ms resolution',
                      'runs': 'One warmup then five measured runs; one process per query/thread count; t1 and t24',
                      'timed_scope': 'Complete authored SELECT in CLI .mode trash. No answer CSV formatting, serialization, download or comparison. Intrinsic engine result materialization/consumption remains part of query execution.',
                      'loading_excluded': 'Database creation/export, process startup/database opening and SET threads excluded. Query-required scanning, decompression and reads remain timed; same-query warmup precedes samples.',
                      'aggregation': 'Median of five measured samples, excluding warmup',
                      'historical_csv_timings': f'performance_duck_sf{sf}.json',
                      'script': 'experiments/gpu/compare/performance_duck.py'},
                  'results': results}
        (output / f'performance_duck_trash_sf{sf}.json').write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
