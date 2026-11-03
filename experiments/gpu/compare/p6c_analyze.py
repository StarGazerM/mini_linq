#!/usr/bin/env python3
"""Analyze rotated pod-bench.json files with paired geometric-mean ratio intervals."""
import argparse
from itertools import product
import json
import math
from pathlib import Path
import random
import statistics


def percentile(sorted_values, probability):
    position = (len(sorted_values) - 1) * probability
    lower = int(position)
    upper = min(lower + 1, len(sorted_values) - 1)
    return sorted_values[lower] + (sorted_values[upper] - sorted_values[lower]) * (position - lower)


def bootstrap_ratios(ratios, seed=20261103):
    """Percentile bootstrap of the geometric mean of whole-process paired ratios."""
    if not ratios:
        return dict(pairs=0, geometric_mean=None, ci95=None, resamples=0, method='unavailable', seed=seed)
    if any(not math.isfinite(value) or value <= 0 for value in ratios):
        raise ValueError('paired ratios must be finite and positive')
    logs = [math.log(value) for value in ratios]
    count = len(logs)
    # n**n <= 1,000,000 iff 1 <= n <= 7; avoid computing huge integers.
    exhaustive = count <= 7
    rng = random.Random(seed)
    samples = (product(logs, repeat=count) if exhaustive else
               ([logs[rng.randrange(count)] for _ in range(count)] for _ in range(20000)))
    means = sorted(math.fsum(sample) / count for sample in samples)
    return dict(pairs=count, geometric_mean=math.exp(math.fsum(logs) / count),
                ci95=[math.exp(percentile(means, p)) for p in (0.025, 0.975)],
                resamples=len(means), method='exhaustive' if exhaustive else 'monte_carlo', seed=seed)


def ranges(records, key):
    values = [record.get('rusage_delta', {}).get(key) for record in records]
    values = [value for value in values if value is not None]
    return [min(values), max(values)] if values else None


def analyze(paths, reference=0, control=None, seed=20261103):
    documents = [json.loads(path.read_text()) for path in paths]
    count = len(documents[0]['bundles'])
    if any(len(doc['bundles']) != count for doc in documents):
        raise ValueError('inputs must have the same bundle index assignments/count')
    if not 0 <= reference < count or (control is not None and not 0 <= control < count):
        raise ValueError('reference/control bundle index is out of range')
    if control == reference:
        raise ValueError('layout control must differ from reference')
    groups = {}
    excluded = []
    for session, document in enumerate(documents):
        for record in document['processes']:
            if record['warmup']:
                continue
            if (record.get('failed') or record.get('exit_code') != 0 or not record.get('exact')
                    or not record.get('per_query')
                    or any(row.get('exact') is not True for row in record['per_query'].values())):
                excluded.append(dict(input=session, round=record['round'], bundle=record['bundle'],
                                     scale=record['scale'], reason='failed or inexact process'))
                continue
            pair = (session, record['round'])
            for query, result in record['per_query'].items():
                for metric in ('total', 'kernels', 'process_wall'):
                    value = (record['process_seconds'] if metric == 'process_wall'
                             else (result.get('median_ms') or {}).get(metric))
                    key = (query, record['scale'], document['mode'], metric)
                    members = groups.setdefault(key, {})
                    samples = members.setdefault(record['bundle'], {})
                    if value is None or not math.isfinite(value) or value <= 0:
                        excluded.append(dict(input=session, round=record['round'], bundle=record['bundle'],
                                             scale=record['scale'], query=query, metric=metric,
                                             reason='missing, nonfinite or nonpositive metric'))
                        continue
                    if pair in samples:
                        raise ValueError(f'duplicate paired observation: {key}, bundle {record["bundle"]}, {pair}')
                    samples[pair] = (value, record)
    rows = []
    for (query, scale, mode, metric), members in sorted(groups.items()):
        reference_samples = members.get(reference, {})
        group_rows = []
        for index in range(count):
            samples = members.get(index, {})
            paired = sorted(samples.keys() & reference_samples.keys())
            ratios = [samples[pair][0] / reference_samples[pair][0] for pair in paired]
            summary = bootstrap_ratios(ratios, seed)
            records = [record for value, record in samples.values()]
            row = dict(query=query, scale=scale, mode=mode, metric=metric,
                       unit='seconds' if metric == 'process_wall' else 'ms',
                       bundle=index, role='reference' if index == reference else
                       'layout-control' if index == control else 'candidate',
                       process_count=len(samples),
                       median=statistics.median(value for value, record in samples.values()) if samples else None,
                       paired_ratios=[dict(input=pair[0], round=pair[1], ratio=ratio)
                                      for pair, ratio in zip(paired, ratios)],
                       unpaired_count=len(samples) - len(paired),
                       reference_unpaired_count=len(reference_samples) - len(paired),
                       minflt_range=ranges(records, 'minflt'), majflt_range=ranges(records, 'majflt'),
                       maxrss_delta_kib_range=ranges(records, 'maxrss'), **summary)
            group_rows.append(row)
        control_row = next((row for row in group_rows if row['bundle'] == control), None)
        for row in group_rows:
            row['control_ci95'] = control_row['ci95'] if control_row else None
        rows.extend(group_rows)
    return dict(version=1, inputs=[str(path.resolve()) for path in paths], reference=reference,
                control=control, seed=seed, rows=rows, excluded=excluded,
                bundles_by_input=[doc['bundles'] for doc in documents],
                not_run_by_input=[doc.get('not_run', []) for doc in documents],
                notes=['Bundle indices must identify the same variants across input files.',
                       'Warmups and failed/inexact processes are excluded; pairs never cross input files.',
                       'Process wall covers all requested queries, including load/transfer and oracle work; it is not a per-query resident time.',
                       'Resident total/kernels are medians of process medians, not pooled iteration timings.',
                       'RSS deltas measure changes in cumulative child high-water RSS, not per-process peaks.'])


def markdown(report):
    lines = [f'Reference: bundle {report["reference"]}; layout control: {report["control"]}; seed: {report["seed"]}.', '',
             '| Query | Scale | Mode | Metric | Bundle / role | Median | Pairs | Ratio [95% CI] | Control CI | Minor faults | Major faults |',
             '|---|---|---|---|---|---:|---:|---|---|---|---|']

    def number(value):
        return 'n/a' if value is None else f'{value:.6g}'

    def interval(value):
        return 'n/a' if value is None else f'[{number(value[0])}, {number(value[1])}]'

    for row in report['rows']:
        lines.append(f'| {row["query"]} | {row["scale"]} | {row["mode"]} | {row["metric"]} ({row["unit"]}) '
                     f'| {row["bundle"]} / {row["role"]} | {number(row["median"])} | {row["pairs"]} '
                     f'| {number(row["geometric_mean"])} {interval(row["ci95"])} '
                     f'| {interval(row["control_ci95"])} | {interval(row["minflt_range"])} '
                     f'| {interval(row["majflt_range"])} |')
    lines += ['', f'Excluded observations: {len(report["excluded"])}; '
              f'unrun processes: {sum(len(items) for items in report["not_run_by_input"])}.', '']
    lines.extend('- ' + note for note in report['notes'])
    return '\n'.join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('inputs', type=Path, nargs='+')
    parser.add_argument('--reference', type=int, default=0)
    parser.add_argument('--control', type=int)
    parser.add_argument('--seed', type=int, default=20261103)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    try:
        report = analyze(args.inputs, args.reference, args.control, args.seed)
    except ValueError as error:
        parser.error(str(error))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(markdown(report))


if __name__ == '__main__':
    main()
