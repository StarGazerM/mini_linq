#!/usr/bin/env python3
"""Exact returned-volume, helper-repetition and run-fence-domain counts from a joint trace."""
import argparse
from collections import Counter
import json
from pathlib import Path
import numpy as np


def analyze(trace):
    descriptions, launches = {}, []
    for line in (trace / 'events.jsonl').open():
        event = json.loads(line)
        if event['event'] == 'snapshot_data':
            descriptions[event['snapshot']] = event
        elif event['event'] == 'launch':
            launches.append(event)
    stats, extrema = Counter(), {}
    for event in launches:
        records = np.fromfile(trace / event['file'], dtype='<u8').reshape(-1, 16)
        for site in event['sites']:
            probes = records[records[:, 0] == site['site']]
            if not len(probes):
                continue
            lengths = probes[:, 7] - probes[:, 6]
            stats['probes'] += len(probes)
            stats['returned_rows'] += int(lengths.sum())
            stats['enumerated_rows'] += event['scan_rows'][site['site']]
            if site['membership']:
                stats['membership_probes'] += len(probes)
            else:
                stats['range_returned_rows'] += int(lengths.sum())
            if site['negated']:
                stats['anti_join_probes'] += len(probes)
            if site.get('nonradix') or 'whole_source' in site:
                continue
            snapshot = int(probes[0, 4])
            description = descriptions[snapshot]
            if not np.all(probes[:, 4] == snapshot):
                raise ValueError('site spans snapshots')
            fields, width = description['fields'], description['word_bytes']
            for run in np.unique(probes[:, 5]):
                cohort = probes[probes[:, 5] == run]
                if site['depth'] == 0:
                    unique, indices = np.unique(cohort[:, 1], return_index=True)
                    stats['root_probes'] += len(cohort)
                    stats['root_repeated_helper_probes'] += len(cohort) - len(unique)
                    stats['root_returned_rows'] += int((cohort[:, 7] - cohort[:, 6]).sum())
                    stats['root_source_deduplicated_returned_rows'] += int((cohort[indices, 7] - cohort[indices, 6]).sum())
                stats['out_of_domain_probes'] += int((cohort[:, 15] >> np.uint64(63)).sum())
                misses = cohort[cohort[:, 6] == cohort[:, 7]]
                rows = int(cohort[0, 12])
                compressed = rows > 32768
                if not rows:
                    stats['empty_run_misses'] += len(misses)
                    continue
                if not len(misses):
                    continue
                stats['nonempty_radix_misses'] += len(misses)
                if compressed:
                    stats['nonempty_compressed_misses'] += len(misses)
                key = (snapshot, int(run))
                if key not in extrema:
                    with (trace / f'snapshot-{snapshot}-{int(run)}.bin').open('rb') as data:
                        first = int.from_bytes(data.read(width), 'little')
                        data.seek((rows - 1) * width)
                        last = int.from_bytes(data.read(width), 'little')
                    extrema[key] = first, last
                first, last = extrema[key]
                for bound in np.unique(misses[:, 15]):
                    if int(bound) >> 63:
                        continue
                    selected = misses[misses[:, 15] == bound]
                    less = np.zeros(len(selected), dtype=bool)
                    greater = less.copy()
                    equal_first = np.ones(len(selected), dtype=bool)
                    equal_last = equal_first.copy()
                    for i, field in enumerate(fields[:int(bound)]):
                        shift, bits = field[:2]
                        signed = field[2] if len(field) > 2 else 0
                        value = selected[:, 8 + i] & np.uint64((1 << bits) - 1)
                        if signed:
                            value = value ^ np.uint64(1 << (bits - 1))
                        a = (first >> shift) & ((1 << bits) - 1)
                        b = (last >> shift) & ((1 << bits) - 1)
                        less |= equal_first & (value < a)
                        greater |= equal_last & (value > b)
                        equal_first &= value == a
                        equal_last &= value == b
                    stats['below_first_fence_misses'] += int(less.sum())
                    stats['above_last_row_misses'] += int(greater.sum())
                    if compressed:
                        stats['compressed_below_first_fence_misses'] += int(less.sum())
                        stats['compressed_above_last_row_misses'] += int(greater.sum())
    return dict(stats)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('trace', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    result = analyze(args.trace)
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))
