#!/usr/bin/env python3
"""Join admission epochs to immutable snapshots; optionally form a separately labelled batched model."""
import argparse
import collections
import hashlib
import itertools
import json
import os
from pathlib import Path
import numpy as np


def events(root):
    with (root / 'events.jsonl').open() as source:
        for line in source:
            yield json.loads(line)


def projection_origin(admission, trace, record, description, states):
    """Prove a static negative projection equals an admitted relation projection.

    This is a byte-exact content proof, not a guessed cross-run snapshot number.
    Candidate constant filters are restricted only by the necessary row count.
    """
    width = description['word_bytes']
    target = np.fromfile(trace / record['file'], dtype=f'V{width}')
    target.sort()
    fields = description['fields']
    for state in sorted(states.values(), key=lambda s: s['final_rows']):
        if state['role'] != 'canonical' or state['final_rows'] < len(target) or len(state['fields']) < len(fields):
            continue
        source = np.concatenate([np.fromfile(admission / b['fresh']['file'], dtype='<u8').reshape(-1, state['word_bytes']//8)
                                 for b in state['batches']])
        values = []
        for field in state['fields']:
            shift, bits = field['shift'], field['bits']
            value = source[:, shift//64] >> np.uint64(shift%64)
            if shift%64 + bits > 64:
                value = value | source[:, shift//64+1] << np.uint64(64-shift%64)
            values.append(value & np.uint64((1 << bits)-1))
        for selected in itertools.permutations(range(len(values)), len(fields)):
            if any(state['fields'][column]['bits'] != field[1] for column, field in zip(selected, fields)):
                continue
            packed = np.zeros((len(source), width//8), dtype='<u8')
            for column, field in zip(selected, fields):
                shift, bits = field[:2]
                packed[:, shift//64] |= values[column] << np.uint64(shift%64)
                if shift%64 + bits > 64:
                    packed[:, shift//64+1] |= values[column] >> np.uint64(64-shift%64)
            projected = packed.view(f'V{width}').reshape(-1)
            if np.array_equal(np.unique(projected), target):
                return dict(snapshot=record['snapshot'], state=state['id'], relation=state['relation'], columns=list(selected), filter=None, rows=len(target))
            for other in set(range(len(values))) - set(selected):
                candidates, counts = np.unique(values[other], return_counts=True)
                for value in candidates[counts >= len(target)]:
                    if np.array_equal(np.unique(projected[values[other] == value]), target):
                        return dict(snapshot=record['snapshot'], state=state['id'], relation=state['relation'], columns=list(selected),
                                    filter=dict(column=other, encoded_value=int(value)), rows=len(target))
    raise ValueError(('negative projection has no exact admitted source', record))


def prepare(admission, trace, output):
    states = {}
    pending = {}
    epochs = {}
    fresh_hashes = collections.defaultdict(list)
    for event in sorted(events(admission), key=lambda e: e.get('seq', -1)):
        kind = event['event']
        if kind == 'register':
            states[event['id']] = {k: event[k] for k in ('id', 'relation', 'role', 'order', 'word_bytes', 'fields')}
            states[event['id']].update(final_rows=0, batches=[])
        elif kind == 'candidates':
            pending[event['id']] = dict(file=event['file'], rows=event['rows'])
        elif kind == 'fresh':
            state = states[event['id']]
            batch = {k: event[k] for k in ('seq', 'stratum', 'phase', 'round')}
            batch['fresh'] = dict(file=event['file'], rows=event['rows'])
            batch['candidates'] = pending.pop(event['id'], None)
            if state['role'] == 'canonical' and batch['candidates'] is None:
                # Seed loading adopts its already-sorted input without a separate candidate event.
                if event['phase'] != 'seed':
                    raise ValueError(('canonical candidates absent', event))
                batch['candidates'] = batch['fresh']
            state['batches'].append(batch)
            state['final_rows'] += event['rows']
            key = (event['id'], event['stratum'], event['phase'], event['round'])
            if key in epochs:
                raise ValueError(('ambiguous admission epoch', key))
            epochs[key] = event
            data = (admission / event['file']).read_bytes()
            if len(data) != event['rows'] * state['word_bytes']:
                raise ValueError(('partial admission rows', event))
            fresh_hashes[hashlib.sha256(data).hexdigest()].append(event['seq'])
    manifest = dict(source='joint DOOP xalan admissions and rule probes; no cross-run epoch inference', states=list(states.values()))
    snapshots, descriptions, joins = {}, {}, {}
    probes = scans = launches = 0
    negative_snapshots = set()
    for event in events(trace):
        if event['event'] == 'snapshot':
            snapshots[event['snapshot']] = event
        elif event['event'] == 'snapshot_data':
            descriptions[event['snapshot']] = event
        elif event['event'] == 'admission_snapshot':
            key = (event['state'], event['stratum'], event['phase'], event['round'])
            fresh = epochs[key]
            joins[event['snapshot']] = fresh
        elif event['event'] == 'launch':
            scans += sum(event['scan_rows'])
            probes += event['records'] - sum(event['scan_rows'])
            launches += 1
            bindings = {site: snapshot for site, snapshot, _ in event['bindings']}
            for site in event['sites']:
                if '__cq_datalog_keys_' in site.get('view', ''):
                    negative_snapshots.add(bindings[site['site']])
    verified = 0
    delta_runs = 0
    unmatched = []
    for snapshot, metadata in snapshots.items():
        description = descriptions[snapshot]
        width = description['word_bytes']
        files = [trace / f'snapshot-{snapshot}-{run}.bin' for run in range(len(metadata['runs']))]
        for file, rows in zip(files, metadata['runs']):
            if file.stat().st_size != rows * width:
                raise ValueError(('partial snapshot', snapshot, file))
        if snapshot in joins:
            fresh = joins[snapshot]
            if metadata['runs'] != fresh['runs']:
                raise ValueError(('run geometry differs from admission', snapshot, metadata['runs'], fresh['runs']))
            state = states[fresh['id']]
            # Bytewise order is sufficient for exact set equality; structured-field
            # comparison is unnecessarily expensive for the repeated large epochs.
            dtype = np.dtype('<u8') if width == 8 else np.dtype(f'V{width}')
            actual = np.concatenate([np.fromfile(file, dtype=dtype) for file in files]) if files else np.empty(0, dtype=dtype)
            admitted = [np.fromfile(admission / batch['fresh']['file'], dtype=dtype)
                        for batch in state['batches'] if batch['seq'] <= fresh['seq']]
            expected = np.concatenate(admitted) if admitted else np.empty(0, dtype=dtype)
            actual.sort(); expected.sort()
            if not np.array_equal(actual, expected):
                raise ValueError(('snapshot contents differ from admission epoch', snapshot))
            verified += 1
        else:
            for file in files:
                digest = hashlib.sha256(file.read_bytes()).hexdigest()
                if digest in fresh_hashes:
                    delta_runs += 1
                else:
                    unmatched.append(dict(snapshot=snapshot, file=file.name, rows=file.stat().st_size // width))
    projections = []
    remaining = []
    for record in unmatched:
        if record['snapshot'] in negative_snapshots:
            projections.append(projection_origin(admission, trace, record, descriptions[record['snapshot']], states))
        else:
            remaining.append(record)
    unmatched = remaining
    if unmatched:
        raise ValueError(('snapshot runs lacking a verified admission origin', unmatched))
    captured_batches = sum(len(s['batches']) for s in states.values())
    # Negative key sets are constructed once from a completed lower stratum.
    # Include their real captured contents in E/P/C admission, rather than
    # silently charging their packed reads but omitting their construction.
    scheduling = [(b['seq'], 0, state['id'], b) for state in states.values() for b in state['batches']]
    for projection in projections:
        snapshot = projection['snapshot']
        description = descriptions[snapshot]
        origin = states[projection['state']]
        last = max(origin['batches'], key=lambda b: b['seq'])
        sid = max(states) + 1
        file = os.path.relpath(trace / f'snapshot-{snapshot}-0.bin', admission)
        batch = dict(seq=0, stratum=last['stratum'], phase='negative_projection', round=0,
                     fresh=dict(file=file, rows=projection['rows']), candidates=None)
        states[sid] = dict(id=sid, relation=f'negative_projection_{snapshot}', role='layout',
                           order=list(range(len(description['fields']))), word_bytes=description['word_bytes'],
                           fields=[dict(shift=f[0], bits=f[1]) for f in description['fields']],
                           final_rows=projection['rows'], batches=[batch])
        scheduling.append((last['seq'], 1, sid, batch))
    for seq, (_, _, _, batch) in enumerate(sorted(scheduling, key=lambda item: item[:3])):
        batch['seq'] = seq
    manifest['states'] = list(states.values())
    (admission / 'manifest.json').write_text(json.dumps(manifest))
    summary = dict(launches=launches, probes=probes, enumerated_rows=scans, snapshots=len(snapshots),
                   exact_admission_snapshot_joins=verified, exact_fresh_run_matches=delta_runs,
                   negative_projection_origins=projections, unmatched_runs=unmatched,
                   captured_admission_batches=captured_batches, replayed_admission_batches=sum(len(s['batches']) for s in states.values()))
    output.write_text(json.dumps(summary, indent=2))
    print(json.dumps(summary))


def batch(trace, target):
    if target.exists():
        raise ValueError('batched trace must be a fresh directory')
    target.mkdir(parents=True)
    metadata = []
    groups = {}
    legacy = {}
    for event in events(trace):
        if event['event'] == 'legacy_data':
            legacy[(event['launch'], event['site'])] = event
        if event['event'] in ('snapshot', 'snapshot_data', 'name'):
            metadata.append(event)
        if event['event'] != 'launch':
            continue
        records = np.fromfile(trace / event['file'], dtype='<u8').reshape(-1, 16)
        for site in event['sites']:
            sid = site['site']
            selected = records[(records[:, 0] & np.uint64((1 << 63) - 1)) == sid]
            if not len(selected):
                continue
            # Legacy/scalar provider addresses can be recycled. Never merge their states by pointer.
            key = (event['kernel'], sid, int(selected[0, 4]), event['seq'] if site.get('nonradix') or 'whole_source' in site else None)
            if key not in groups:
                seq = len(groups)
                launch = dict(event, seq=seq, file=f'{seq}.bin', records=0,
                              sites=[dict(site, replay_preordered=True)], scan_rows=[0] * len(event['scan_rows']), original_launches=0)
                groups[key] = launch
                if site.get('nonradix'):
                    metadata.append(dict(legacy[(event['seq'], sid)], launch=seq))
            launch = groups[key]
            if site['depth'] == 0:
                scans = selected[:, 0] >> np.uint64(63)
                selected = selected[np.lexsort((np.where(scans, 0, selected[:, 1]), np.where(scans, 0, selected[:, 5]), np.where(scans, 0, selected[:, 4]), scans))]
            with (target / launch['file']).open('ab') as stream:
                selected.tofile(stream)
            launch['records'] += len(selected)
            launch['scan_rows'][sid] += event['scan_rows'][sid]
            launch['original_launches'] += 1
    for file in trace.iterdir():
        if file.name.startswith(('snapshot-', 'legacy-')):
            (target / file.name).symlink_to(file.resolve())
    with (target / 'events.jsonl').open('w') as stream:
        for event in [dict(event='replay_model', model='batched same immutable snapshot and site across launches; different execution model'), *metadata, *groups.values()]:
            stream.write(json.dumps(event) + '\n')
    print(json.dumps(dict(batched_site_launches=len(groups), original_site_launches=sum(g['original_launches'] for g in groups.values()))))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    p = sub.add_parser('prepare'); p.add_argument('admission', type=Path); p.add_argument('trace', type=Path); p.add_argument('output', type=Path)
    p = sub.add_parser('batch'); p.add_argument('trace', type=Path); p.add_argument('target', type=Path)
    args = parser.parse_args()
    if args.command == 'prepare':
        prepare(args.admission, args.trace, args.output)
    else:
        batch(args.trace, args.target)
