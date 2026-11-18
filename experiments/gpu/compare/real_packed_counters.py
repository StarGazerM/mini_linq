#!/usr/bin/env python3
"""Profile one recorded launch at each of the three heaviest measured replay sites."""
import argparse
import collections
import csv
import fcntl
import io
import json
from pathlib import Path
import subprocess
from real_packed_replay import tenants

METRICS = ['smsp__inst_executed.sum', 'smsp__thread_inst_executed_per_inst_executed.ratio',
           'smsp__warp_issue_stalled_long_scoreboard_per_warp_active.pct',
           'l1tex__t_sector_hit_rate.pct', 'lts__t_sector_hit_rate.pct',
           'launch__registers_per_thread', 'sm__warps_active.avg.pct_of_peak_sustained_active',
           'gpu__time_duration.sum', 'launch__grid_size', 'launch__block_size']
L2_METRICS = ['lts__t_sectors_op_read.sum', 'lts__t_sectors_op_read_lookup_hit.sum']


def capture(prefix, command, names, resume):
    complete = all(prefix.with_suffix(suffix).exists() for suffix in ('.csv', '.ncu-rep', '.before', '.after', '.replay.json'))
    if resume and complete:
        if prefix.with_suffix('.before').read_text().strip() or prefix.with_suffix('.after').read_text().strip():
            raise RuntimeError(f'cannot reuse a contended capture: {prefix}')
    else:
        with open('/tmp/mini_linq-gpu.lock', 'w') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            before = tenants(); prefix.with_suffix('.before').write_text(before)
            if before:
                raise RuntimeError(f'GPU not quiet: {before}')
            try:
                run = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
                prefix.with_suffix('.csv').write_text(run.stdout)
            finally:
                after = tenants(); prefix.with_suffix('.after').write_text(after)
            if after:
                raise RuntimeError(f'GPU not quiet after counter capture: {after}')
            if run.returncode:
                raise RuntimeError(f'NCU failed; inspect {prefix}.csv')
    lines = prefix.with_suffix('.csv').read_text().splitlines()
    header = next(i for i, line in enumerate(lines) if line.startswith('"ID",'))
    metrics, units = {}, {}
    for row in csv.DictReader(io.StringIO('\n'.join(lines[header:]))):
        if row.get('Metric Name') in names:
            metrics[row['Metric Name']] = dict(value=row['Metric Value'], unit=row['Metric Unit'])
        elif not row.get('ID'):
            units = row
        else:
            for name in names:
                if name in row:
                    metrics[name] = dict(value=row[name], unit=units.get(name, ''))
    missing = set(names) - metrics.keys()
    if missing:
        raise RuntimeError(f'missing counters: {missing}')
    return metrics


def number(metrics, key):
    return float(metrics[key]['value'].replace(',', ''))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('binary', 'trace', 'sample', 'output', 'ncu'):
        parser.add_argument(name, type=Path)
    parser.add_argument('--resume', action='store_true', help='reuse completed, uncontended counter captures')
    args = parser.parse_args()
    rows = json.loads(args.sample.read_text())['sites']
    sites = collections.defaultdict(list)
    for row in rows:
        sites[f"{row['kernel']}/{row['site']['site']}"].append(row)
    heavy = sorted(sites, key=lambda key: sum(r['gpu_ns'][0] / r['repeats'] for r in sites[key]), reverse=True)[:3]
    args.output.mkdir(parents=True, exist_ok=True)
    result = []
    for rank, site in enumerate(heavy, 1):
        representative = max(sites[site], key=lambda r: r['probes'] + r['enumerated_rows'])
        for mode, nvtx in [('E', 'real_engine'), ('P', 'real_plain'), ('C', 'real_packed')]:
            prefix = args.output / f'site-{rank}-{mode}'
            command = [str(args.ncu), '--target-processes', 'all', '--nvtx', '--nvtx-include', f'real_site/{nvtx}/',
                       '--launch-count', '1', '--cache-control', 'none', '--clock-control', 'none',
                       '--metrics', ','.join(METRICS), '--force-overwrite', '--export', str(prefix), '--csv', '--page', 'raw',
                       str(args.binary.resolve()), '--real-trace', str(args.trace.resolve()), '--real-site', site,
                       '--real-launch', str(representative['launch']), '--real-reads-only', '--output', str(prefix) + '.replay.json']
            metrics = capture(prefix, command, METRICS, args.resume)
            # The broad multipass derived L2 percentage can exceed 100 on short
            # launches. Collect its read hit/total counts in isolation instead.
            l2_prefix = args.output / f'site-{rank}-{mode}-l2'
            l2_command = command.copy()
            l2_command[l2_command.index('--metrics') + 1] = ','.join(L2_METRICS)
            l2_command[l2_command.index('--export') + 1] = str(l2_prefix)
            l2_command[l2_command.index('--output') + 1] = str(l2_prefix) + '.replay.json'
            l2_metrics = capture(l2_prefix, l2_command, L2_METRICS, args.resume)
            l2_rate = 100 * number(l2_metrics, L2_METRICS[1]) / number(l2_metrics, L2_METRICS[0])
            if not 0 <= l2_rate <= 100:
                raise RuntimeError(f'inconsistent isolated L2 counters: {l2_metrics}')
            probes = representative['probes']
            result.append(dict(rank=rank, site=site, rule=representative['rule'], launch=representative['launch'], mode=mode,
                               probes=probes, enumerated_rows=representative['enumerated_rows'], metrics=metrics,
                               l2_read_metrics=l2_metrics, l2_read_hit_percent=l2_rate,
                               warp_instructions_per_probe=number(metrics, 'smsp__inst_executed.sum') / probes,
                               active_thread_instructions_per_probe=number(metrics, 'smsp__inst_executed.sum') * number(metrics, 'smsp__thread_inst_executed_per_inst_executed.ratio') / probes))
            (args.output / 'summary.json').write_text(json.dumps(result, indent=2))
            print(f'captured site {rank} {mode}', flush=True)


if __name__ == '__main__':
    main()
