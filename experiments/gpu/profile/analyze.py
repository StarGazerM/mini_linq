#!/usr/bin/env python3
"""Analyze Nsight Systems SQLite exports, with or without MiniLinq NVTX.

All times in the export are nanoseconds. GPU busy is the union of kernel,
memcpy and memset intervals, not the sum. Rounds use host NVTX boundaries;
launch-correlated kernel counts remain correct when kernels outlive a range.
"""
import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path
import re
import sqlite3

SYNC = re.compile(r'(?:cuda|cu)(?:StreamSynchronize|EventSynchronize|DeviceSynchronize|CtxSynchronize|MemcpyDtoH|MemcpyDtoHAsync|Memcpy)$')


def is_sync(api, readback_ids):
    name = re.sub(r'(?:_v\d+|_ptsz|_ptds)+$', '', api['name'])
    return bool(SYNC.fullmatch(name)) or (
        name in ('cudaMemcpyAsync', 'cuMemcpyAsync') and api.get('correlationId') in readback_ids)


def merged(intervals):
    result = []
    for start, end in sorted(intervals):
        if end <= start:
            continue
        if result and start <= result[-1][1]:
            result[-1] = (result[-1][0], max(end, result[-1][1]))
        else:
            result.append((start, end))
    return result


def duration(intervals):
    return sum(b - a for a, b in merged(intervals))


def clipped(events, windows):
    return [(max(e['start'], a), min(e['end'], b)) for a, b in windows
            for e in events if e['start'] < b and e['end'] > a]


def stats(values):
    values = sorted(values)
    return {'count': len(values), 'total_ms': sum(values) / 1e6,
            'mean_us': sum(values) / max(1, len(values)) / 1e3,
            'p50_us': values[len(values) // 2] / 1e3 if values else 0,
            'p95_us': values[min(len(values) - 1, int(len(values) * .95))] / 1e3 if values else 0,
            'max_us': max(values, default=0) / 1e3}


def idle_intervals(events, windows):
    result = []
    for a, b in windows:
        cursor = a
        for start, end in merged(clipped(events, [(a, b)])):
            if start > cursor:
                result.append((cursor, start))
            cursor = end
        if cursor < b:
            result.append((cursor, b))
    return result


def gaps(events, windows):
    return [b - a for a, b in idle_intervals(events, windows)]


def intersection_duration(left, right):
    left, right = merged(left), merged(right)
    i = j = total = 0
    while i < len(left) and j < len(right):
        a, b = left[i]
        c, d = right[j]
        total += max(0, min(b, d) - max(a, c))
        if b < d:
            i += 1
        else:
            j += 1
    return total


def concurrency(kernels, windows):
    total = duration(windows)
    active_time = overlap = weighted = 0
    maximum = 0
    for a, b in windows:
        points = defaultdict(list)
        for k in kernels:
            if k['start'] < b and k['end'] > a:
                stream = (k.get('deviceId', 0), k.get('contextId', 0), k.get('streamId', 0))
                points[max(a, k['start'])].append((stream, 1))
                points[min(b, k['end'])].append((stream, -1))
        streams = Counter()
        count = 0
        previous = a
        for time, changes in sorted(points.items()):
            dt = time - previous
            active = sum(v > 0 for v in streams.values())
            weighted += active * dt
            active_time += dt if count else 0
            overlap += dt if count >= 2 else 0
            maximum = max(maximum, active)
            for stream, delta in changes:
                streams[stream] += delta
                count += delta
            previous = time
    return {'max_streams': maximum, 'avg_streams_window': weighted / max(1, total),
            'avg_streams_kernel_busy': weighted / max(1, active_time),
            'kernel_overlap_pct': overlap / max(1, total) * 100}


def analyze(path, range_name, window_index=None):
    db = sqlite3.connect(f'file:{Path(path).resolve()}?mode=ro', uri=True)
    db.row_factory = sqlite3.Row
    tables = {r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
    def rows(name):
        return [dict(r) for r in db.execute(f'SELECT * FROM "{name}"')] if name in tables else []
    strings = {r['id']: r['value'] for r in rows('StringIds')}
    def text(row, *keys):
        for key in keys:
            value = row.get(key)
            if value is not None:
                return strings.get(value, str(value)) if not isinstance(value, str) else value
        return '<unknown>'
    kernels = rows('CUPTI_ACTIVITY_KIND_KERNEL') or rows('CUPTI_ACTIVITY_KIND_CONCURRENT_KERNEL')
    if not kernels:
        raise ValueError('No CUDA kernel events in export')
    for k in kernels:
        k['name'] = text(k, 'demangledName', 'shortName', 'name')
    copies = rows('CUPTI_ACTIVITY_KIND_MEMCPY')
    readback_ids = {c.get('correlationId') for c in copies if c.get('copyKind') in (2, 4, 12)}
    gpu = kernels + copies + rows('CUPTI_ACTIVITY_KIND_MEMSET')
    apis = rows('CUPTI_ACTIVITY_KIND_RUNTIME') + rows('CUPTI_ACTIVITY_KIND_DRIVER')
    for api in apis:
        api['name'] = text(api, 'nameId', 'name')
    ranges = rows('NVTX_EVENTS')
    for r in ranges:
        r['name'] = text(r, 'text', 'textId')
    ranges = [r for r in ranges if r.get('end') is not None and r['end'] > r['start']]
    windows = merged((r['start'], r['end']) for r in ranges if r['name'] == range_name)
    scope = f'NVTX {range_name}'
    if not windows:
        windows = [(min(k['start'] for k in gpu), max(k['end'] for k in gpu))]
        scope = 'first-to-last GPU activity (no matching NVTX range)'
    if window_index is not None:
        windows = [windows[window_index]]
        scope += f' window {window_index}'
    def intersects(event, spans=windows):
        return any(event['start'] < b and event['end'] > a for a, b in spans)
    selected = [k for k in kernels if intersects(k)]
    selected_apis = [a for a in apis if intersects(a)]
    # Runtime and driver API wrappers may nest. Keep individual calls for counts,
    # but use interval union for host API time; never add nested call durations.
    syncs = [a for a in selected_apis if is_sync(a, readback_ids)]
    correlations = defaultdict(list)
    for api in apis:
        if 'correlationId' in api:
            correlations[api['correlationId']].append(api)
    def launched_in(k, r):
        return any(r['start'] <= a['start'] < r['end'] and
                   (r.get('globalTid') is None or a.get('globalTid') == r.get('globalTid'))
                   for a in correlations.get(k.get('correlationId'), []))
    rounds = []
    for r in sorted((r for r in ranges if r['name'].startswith('round/') and intersects(r)), key=lambda r: r['start']):
        rw = [(r['start'], r['end'])]
        rk = [k for k in kernels if launched_in(k, r)]
        rs = [a for a in syncs if r['start'] <= a['start'] < r['end'] and a.get('globalTid') == r.get('globalTid')]
        parents = [p for p in ranges if p['name'].startswith('scc/') and p['start'] <= r['start'] and p['end'] >= r['end'] and p.get('globalTid') == r.get('globalTid')]
        rounds.append({'round': len(rounds), 'name': r['name'], 'scc': parents[0]['name'] if parents else None,
                       'host_ms': (r['end'] - r['start']) / 1e6, 'kernels': len(rk),
                       'sync_calls': len(rs), 'sync_names': dict(Counter(a['name'] for a in rs)),
                       'gpu_idle': stats(gaps(gpu, rw))})
    grouped = defaultdict(list)
    for k in selected:
        grouped[k['name']].append(k)
    top = []
    for name, ks in grouped.items():
        ks.sort(key=lambda k: k['start'])
        representative = sorted(range(len(ks)), key=lambda i: ks[i]['end'] - ks[i]['start'])[len(ks) // 2]
        item = {'name': name, 'count': len(ks), 'total_ms': sum(k['end'] - k['start'] for k in ks) / 1e6,
                'avg_us': sum(k['end'] - k['start'] for k in ks) / len(ks) / 1e3}
        item['representative_launch_skip'] = representative
        for kind in ('grid', 'block'):
            item[kind] = [sum(k.get(kind + axis, 0) for k in ks) / len(ks) for axis in ('X', 'Y', 'Z')]
        top.append(item)
    top.sort(key=lambda k: -k['total_ms'])
    launch_apis = sorted((a for a in selected_apis if re.search(r'(?:LaunchKernel|LaunchCooperativeKernel)', a['name'])), key=lambda a: a['start'])
    launch_gaps = [max(0, b['start'] - a['end']) for a, b in zip(launch_apis, launch_apis[1:])
                   if any(x <= a['start'] <= b['start'] < y for x, y in windows)]
    buckets = [0, 1, 5, 10, 50, 100, 1000, float('inf')]
    histogram = {f'{a:g}..{b:g} us': sum(a <= gap / 1e3 < b for gap in launch_gaps) for a, b in zip(buckets, buckets[1:])}
    wall = duration(windows)
    busy = duration(clipped(gpu, windows))
    idle = idle_intervals(gpu, windows)
    idle_cuda = intersection_duration(idle, clipped(selected_apis, windows))
    idle_sync = intersection_duration(idle, clipped(syncs, windows))
    # Exact CPU-side annotation attribution, independent of asynchronous GPU time.
    phases = defaultdict(list)
    for r in ranges:
        if intersects(r):
            phases[r['name'].split('/')[0] if r['name'].startswith('hole/') else r['name']].extend(clipped([r], windows))
    return {'source': str(Path(path).resolve()), 'scope': scope, 'windows': windows,
            'window_ms': wall / 1e6, 'gpu_busy_ms': busy / 1e6, 'gpu_busy_pct': busy / wall * 100,
            'kernel_busy_ms': duration(clipped(kernels, windows)) / 1e6,
            'kernel_count': len(selected), 'gpu_idle': stats(gaps(gpu, windows)),
            'host_cuda_api_ms': duration(clipped(selected_apis, windows)) / 1e6,
            'host_sync_api_ms': duration(clipped(syncs, windows)) / 1e6,
            'gpu_idle_in_cuda_api_ms': idle_cuda / 1e6,
            'gpu_idle_in_sync_api_ms': idle_sync / 1e6,
            'gpu_idle_outside_cuda_api_ms': (wall - busy - idle_cuda) / 1e6,
            'sync_calls': len(syncs), 'sync_names': dict(Counter(a['name'] for a in syncs)),
            'concurrency': concurrency(selected, windows), 'rounds': rounds,
            'top_kernels': top, 'launch_gaps': stats(launch_gaps), 'launch_gap_histogram': histogram,
            'host_nvtx_ms': {name: duration(spans) / 1e6 for name, spans in phases.items()}}


def markdown(r):
    c = r['concurrency']
    out = [f"# {Path(r['source']).stem}", '', f"Scope: {r['scope']}; {len(r['windows'])} window(s).", '',
           '| Window ms | GPU busy % | Kernel count | Overlap % | Max streams | Avg streams | Sync calls |',
           '|---:|---:|---:|---:|---:|---:|---:|',
           f"| {r['window_ms']:.3f} | {r['gpu_busy_pct']:.2f} | {r['kernel_count']} | {c['kernel_overlap_pct']:.3f} | {c['max_streams']} | {c['avg_streams_window']:.3f} | {r['sync_calls']} |", '',
           f"GPU idle: {r['gpu_idle']['total_ms']:.3f} ms; gap p50/p95/max: {r['gpu_idle']['p50_us']:.3f}/{r['gpu_idle']['p95_us']:.3f}/{r['gpu_idle']['max_us']:.3f} us.",
           f"Host CUDA API interval union: {r['host_cuda_api_ms']:.3f} ms; synchronization/readback API union: {r['host_sync_api_ms']:.3f} ms.", '',
           f"GPU idle in CUDA APIs: {r['gpu_idle_in_cuda_api_ms']:.3f} ms (sync/readback subset {r['gpu_idle_in_sync_api_ms']:.3f} ms); idle outside CUDA APIs: {r['gpu_idle_outside_cuda_api_ms']:.3f} ms.", '',
           '| Kernel | Count | Total ms | Mean us | Mean grid | Mean block |', '|---|---:|---:|---:|---|---|']
    for k in r['top_kernels'][:5]:
        out.append(f"| `{k['name']}` | {k['count']} | {k['total_ms']:.3f} | {k['avg_us']:.3f} | {k['grid']} | {k['block']} |")
    out += ['', 'Launch-gap histogram (API end → next launch start):', '', '```json', json.dumps(r['launch_gap_histogram'], indent=2), '```', '',
            '| Round | SCC | Phase | Kernels | Sync calls | GPU idle ms | Idle p95 us |', '|---:|---|---|---:|---:|---:|---:|']
    for row in r['rounds']:
        out.append(f"| {row['round']} | {row['scc']} | {row['name']} | {row['kernels']} | {row['sync_calls']} | {row['gpu_idle']['total_ms']:.3f} | {row['gpu_idle']['p95_us']:.3f} |")
    if not r['rounds']:
        out += ['', 'Per-round metrics unavailable: capture has no round NVTX ranges.']
    return '\n'.join(out) + '\n'


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('sqlite')
    p.add_argument('--range', default='fixed-point', dest='range_name')
    p.add_argument('--window-index', type=int, help='zero-based chronological matching range')
    p.add_argument('--json', type=Path)
    p.add_argument('--markdown', type=Path)
    args = p.parse_args()
    result = analyze(args.sqlite, args.range_name, args.window_index)
    if args.json:
        args.json.write_text(json.dumps(result, indent=2) + '\n')
    rendered = markdown(result)
    if args.markdown:
        args.markdown.write_text(rendered)
    else:
        print(rendered, end='')


if __name__ == '__main__':
    main()
