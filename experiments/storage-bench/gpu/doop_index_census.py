#!/usr/bin/env python3
"""Measure DOOP EDB/output columns and model every emitted full-key GPU layout.

Inputs are the actual compiler's REL/FULL/DELTA/NEG tab-separated inventory,
a revision-matched GPU_DATALOG_TRACE_MEMORY log, and a full BENCH_CHECK witness.
No engine code or facts are changed. SQL reconstructs only acyclic non-output
relations from the authored rules; it does not substitute synthetic IDB sizes.
Candidate byte counts are serialized-layout estimates, not benchmark timings.
"""
import argparse
from collections import defaultdict
import hashlib
import json
import math
from pathlib import Path
import re

import duckdb
import numpy as np


def split_args(text):
    result, start, depth, quoted, escaped = [], 0, 0, False, False
    for i, char in enumerate(text):
        if quoted:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                quoted = False
        elif char == '"':
            quoted = True
        elif char == '(':
            depth += 1
        elif char == ')':
            depth -= 1
        elif char == ',' and depth == 0:
            result.append(text[start:i].strip())
            start = i + 1
    result.append(text[start:].strip())
    return result


def atom(text):
    match = re.fullmatch(r'(\w+)\((.*)\)', text.strip(), re.S)
    if not match:
        raise ValueError(f'Not a positive relational atom: {text}')
    return match[1], split_args(match[2])


def semantic_domains(con, schemas, rule_text):
    """Shared heading domains induced by actual rule-variable equality."""
    parent = {(name, col): (name, col) for name, types in schemas.items()
              for col in range(len(types))}
    def root(node):
        while parent[node] != node:
            parent[node] = parent[parent[node]]
            node = parent[node]
        return node
    def union(left, right):
        left, right = root(left), root(right)
        if schemas[left[0]][left[1]] != schemas[right[0]][right[1]]:
            raise ValueError('Rule domain joins incompatible native types')
        if left != right:
            parent[max(left, right)] = min(left, right)
    for statement in rule_text.split(';'):
        if ':-' not in statement:
            continue
        head, body = statement.split(':-', 1)
        bindings = {}
        for clause in [head] + split_args(body):
            clause = clause.strip().lstrip('!')
            if clause.startswith('where '):
                # All authored variable-to-variable filters are !=; equality
                # filters compare a variable to a constant, not another column.
                continue
            name, terms = atom(clause)
            for col, term in enumerate(terms):
                if term == '_' or not re.fullmatch(r'[A-Za-z_]\w*', term):
                    continue
                node = (name, col)
                if term in bindings:
                    union(bindings[term], node)
                else:
                    bindings[term] = node
    components = defaultdict(list)
    for node in parent:
        components[root(node)].append(node)
    anchors = [('varpointsto', 0, 'heap'), ('varpointsto', 1, 'variable'),
               ('method_simplename', 0, 'method'), ('isreferencetype', 0, 'type'),
               ('instruction_method', 0, 'instruction'), ('field_declaringtype', 0, 'field'),
               ('method_simplename', 1, 'simple_name'), ('method_descriptor', 1, 'descriptor')]
    domains, column_domain = {}, {}
    for number, members in enumerate(sorted(components.values())):
        label = next((label for name, col, label in anchors if (name, col) in members),
                     f'{members[0][0]}.c{members[0][1]}')
        identity = f'doop.semantic.{number}.{label}'
        union_query = ' UNION ALL '.join(f'SELECT c{col} AS value FROM \"{name}\"' for name, col in members)
        count, low, high = con.execute(
            f'SELECT count(DISTINCT value),min(value),max(value) FROM ({union_query})').fetchone()
        domains[identity] = {
            'label': label, 'columns': [f'{name}.c{col}' for name, col in members],
            'distinct': count, 'canonical_code_min': low, 'canonical_code_max': high,
            'logical_type': schemas[members[0][0]][members[0][1]],
            'new_gpu_mapping_bytes_estimate': 4 * count + 8,
            'mapping_basis': 'estimated sorted u32 canonical-code array plus u64 length; binary search returns shared dense rank and indexing the same array performs inverse lookup; paid once per component, not per set; no O(1) forward lookup assumed',
            'basis': 'measured UNION DISTINCT of all real tuple values in all component columns; component edges are identical rule variables across heads and positive/negative atoms',
            'evidence': ['features/datalog/doop/shared/src/mini_engine.rs:157-586']}
        for member in members:
            column_domain[member] = identity
    return domains, column_domain


def file_sha256(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def ceildiv(n, d):
    return (n + d - 1) // d


def bitmap(bits, rows, metadata=8):
    data = ceildiv(bits, 64) * 8
    rank = (ceildiv(bits, 512) + 1) * 8 if bits else 0
    return {'bits': bits, 'data_bytes': data, 'rank_bytes': rank,
            'bytes': data + rank + metadata, 'membership_only_bytes': data + metadata,
            'with_rank_bytes': data + rank + metadata, 'metadata_bytes': metadata,
            'occupancy': rows / bits if bits else None,
            'basis': 'estimated: u64 words; u64 rank every 512 bits plus terminal included for sorted-position/count reads; count/header separately included'}


def for_cost(a):
    rows, arity = a.shape
    data = 0
    for begin in range(0, rows, 1024):
        block = a[begin:begin + 1024]
        widths = [int(x).bit_length() for x in block.max(axis=0) - block.min(axis=0)]
        data += sum(ceildiv(len(block) * width, 64) * 8 for width in widths)
    blocks = ceildiv(rows, 1024)
    metadata = blocks * arity * 16
    fences = blocks * arity * 8
    return {'bytes': data + metadata + fences + 8, 'data_bytes': data, 'count_bytes': 8,
            'block_metadata_bytes': metadata, 'fence_bytes': fences, 'blocks': blocks,
            'basis': 'estimated from exact lexicographically sorted rows: independent per-column FOR1024; 4-byte base, 4-byte width, 8-byte offset per column/block; first+last complete u32 tuple fences; each bitstream u64-rounded'}


def adaptive_binary(a):
    if a.shape[1] != 2:
        return {'applicable': False, 'bytes': None, 'reason': 'requires binary complete identity'}
    if not len(a):
        return {'applicable': True, 'bytes': 8, 'count_bytes': 8, 'array_chunks': 0, 'bitmap_chunks': 0}
    groups = int(np.count_nonzero(np.r_[True, a[1:, 0] != a[:-1, 0]]))
    chunk_start = np.r_[True, (a[1:, 0] != a[:-1, 0]) | ((a[1:, 1] >> 16) != (a[:-1, 1] >> 16))]
    starts = np.flatnonzero(chunk_start)
    counts = np.diff(np.r_[starts, len(a)])
    dense = counts > 4096
    data = int(np.minimum(2 * counts, 8192).sum())
    directories = groups * 20 + 8 + len(counts) * 24
    rank = int(dense.sum()) * (128 + 1) * 8
    return {'applicable': True, 'bytes': data + directories + rank + 8, 'count_bytes': 8,
            'data_bytes': data, 'directory_bytes': directories,
            'first_keys': groups, 'chunks': len(counts),
            'array_chunks': int((~dense).sum()), 'bitmap_chunks': int(dense.sum()),
            'rank_bytes': rank, 'with_rank_bytes': data + directories + rank + 8,
            'membership_only_bytes': data + directories + 8,
            'basis': 'estimated, not implemented Roaring: shared u32 codes; per-first-key u32 key+u64 chunk offset+u64 cumulative row count (20 bytes) plus terminal offset; each nonempty second-key 2^16 chunk has 24-byte header (high16/type, cardinality, u64 data offset, u64 cumulative rank) and min(2*occupancy,8192) bytes; u64 rank per 512 bits+terminal in bitmap chunks'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, required=True)
    parser.add_argument('--facts', type=Path, required=True)
    parser.add_argument('--layouts', type=Path, required=True)
    parser.add_argument('--witness', type=Path, required=True)
    parser.add_argument('--log', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    macro_path = args.repo / 'features/datalog/doop/shared/src/mini_engine.rs'
    shared_path = args.repo / 'features/datalog/doop/shared/src/lib.rs'
    source = macro_path.read_text()
    shared = shared_path.read_text()
    schemas = {m[1]: re.findall(r'c\d+:\s*(\w+)', m[2]) for m in re.finditer(r'relation\s+(\w+)\(([^;]*)\)(?:\s*=\s*\([^;]*\))?;', source)}
    inventory, relations = [], {}
    for line in args.layouts.read_text().splitlines():
        fields = line.split('\t')
        if fields[0] == 'REL':
            relations[fields[1]] = {'arity': int(fields[2]), 'edb': fields[3] == 'true', 'canonical': int(fields[4])}
        elif fields[0] in ('FULL', 'DELTA', 'NEG'):
            inventory.append({'kind': fields[0], 'layout_id': int(fields[1]), 'relation': fields[2],
                              'order': json.loads(fields[3]), 'lookup_arity': int(fields[4]) if len(fields) > 4 else len(json.loads(fields[3]))})
    if set(relations) != set(schemas):
        raise ValueError('Compiler/source relation coverage mismatch')
    owners = {}
    for number, line in enumerate(args.log.read_text().splitlines(), 1):
        if line.startswith('gpu_datalog_owner\t'):
            values = dict(part.split('=', 1) for part in line.split('\t')[1:])
            owners[values['relation']] = {key: (int(value) if value.isdigit() else value) for key, value in values.items()}
            owners[values['relation']]['source_line'] = number
    codes, strings, numeric_universe = {}, [], set()
    def intern(value):
        if value not in codes:
            codes[value] = len(strings)
            strings.append(value)
        return codes[value]
    con = duckdb.connect()
    con.execute('SET threads=4')
    con.execute("SET memory_limit='12GB'")
    evidence, raw_counts, fact_hashes = {}, {}, {}
    loaded = set()
    def save(name, rows):
        rows = np.asarray(rows, dtype=np.int64).reshape((-1, len(schemas[name])))
        con.register('input_rows', {f'c{i}': rows[:, i] for i in range(rows.shape[1])})
        con.execute(f'CREATE TABLE "{name}" AS SELECT DISTINCT * FROM input_rows')
        con.unregister('input_rows')
        loaded.add(name)
    def load(name, filename):
        rows, digest = [], hashlib.sha256()
        path = args.facts / filename
        with path.open('rb') as stream:
            for line in stream:
                digest.update(line)
                fields = line.decode().rstrip('\n').removesuffix('\r').split('\t')
                if len(fields) != len(schemas[name]):
                    raise ValueError(f'{path}: schema mismatch')
                row = []
                for value, ty in zip(fields, schemas[name]):
                    if ty == 'IStr':
                        row.append(intern(value))
                    else:
                        number = int(value)
                        numeric_universe.add(number)
                        row.append(number)
                rows.append(row)
        raw_counts[name] = len(rows)
        fact_hashes[filename] = digest.hexdigest()
        save(name, rows)
        evidence[name] = [str(path), 'features/datalog/doop/shared/src/loader.rs:59-88']
        print('loaded', name, len(rows), flush=True)
    # Exactly the runtime's first-sight insertion order, including descriptors.
    load('_method', 'Method.facts')
    methods = con.execute('SELECT DISTINCT c0,c2,c4 FROM _method ORDER BY c0,c4,c2').fetchall()
    method_groups = defaultdict(lambda: (set(), set()))
    for method, params, returns in methods:
        method_groups[method][0].add(returns)
        method_groups[method][1].add(params)
    descriptors = [(method, intern(f'{strings[ret]}({strings[param]})'))
                   for method, (returns, params) in sorted(method_groups.items())
                   for ret in sorted(returns) for param in sorted(params)]
    save('method_descriptor', descriptors)
    evidence['method_descriptor'] = ['features/datalog/doop/shared/src/loader.rs:91-117', str(args.facts / 'Method.facts')]
    for name, filename in re.findall(r'(\w+): load!\("([^"]+)"\)', shared):
        load(name, filename)
    constants = list(dict.fromkeys(re.findall(r'sym\("([^"]*)"\)', source)))
    absent_constants = [value for value in constants if value not in codes]
    for value in constants:
        intern(value)
    # Only missing constant codes can depend on generated invocation order.
    witness = json.loads(args.witness.read_text())
    output_names = dict(re.findall(r'relation!\("([^"]+)",\s*(\w+),', shared))
    for label, value in witness.items():
        name = output_names[label]
        rows = value['rows']
        if len(rows) != value['count']:
            raise ValueError(f'{label}: not a full witness')
        if name not in loaded:
            save(name, [[codes[v] for v in row] for row in rows])
            evidence[name] = [str(args.witness) + ':' + label]
        print('witness', name, len(rows), flush=True)
    del witness
    # The remaining non-output IDBs are acyclic positive projections/joins.
    clean = re.sub(r'//[^\n]*', '', source)
    rule_text = clean[clean.index('    istype(class) :-'):clean.index('        };')]
    by_head = defaultdict(list)
    for statement in rule_text.split(';'):
        if ':-' not in statement:
            continue
        head, body = statement.split(':-', 1)
        name, terms = atom(head)
        if name not in loaded:
            by_head[name].append((terms, [atom(item) for item in split_args(body)]))
    while by_head:
        ready = [name for name, rules in by_head.items() if all(dep in loaded for _, atoms in rules for dep, _ in atoms)]
        if not ready:
            raise ValueError(f'Unknown/cyclic non-output IDBs: {list(by_head)}')
        for name in ready:
            selects = []
            for terms, atoms in by_head.pop(name):
                bound, conditions, tables = {}, [], []
                def expr(term):
                    match = re.fullmatch(r'sym\("(.*)"\)', term)
                    return str(codes[match[1]]) if match else bound[term]
                for alias, (dep, terms_in) in enumerate(atoms):
                    tables.append(f'"{dep}" t{alias}')
                    for col, term in enumerate(terms_in):
                        field = f't{alias}.c{col}'
                        if term == '_':
                            continue
                        if term in bound or term.startswith('sym('):
                            conditions.append(f'{field}={expr(term)}')
                        else:
                            bound[term] = field
                selects.append('SELECT DISTINCT ' + ','.join(f'{expr(term)} AS c{i}' for i, term in enumerate(terms)) + ' FROM ' + ','.join(tables) + (' WHERE ' + ' AND '.join(conditions) if conditions else ''))
            con.execute(f'CREATE TABLE "{name}" AS ' + ' UNION '.join(selects))
            loaded.add(name)
            evidence[name] = ['exact SQL evaluation of acyclic authored rules', 'features/datalog/doop/shared/src/mini_engine.rs:157-348']
    symbol_domain = len(strings)
    int_values = sorted(numeric_universe)
    domains = {'doop.process_global_IStr': {'distinct': symbol_domain, 'min': 0, 'max': symbol_domain - 1,
               'basis': 'measured union of all loaded strings, computed method descriptors and authored constants; same process-global interner shared by all IStr columns',
               'new_gpu_mapping_bytes': 0, 'existing_host_string_utf8_bytes': sum(len(s.encode()) for s in strings),
               'existing_host_reverse_vector_live_bytes_estimate': 16 * symbol_domain,
               'existing_host_hashmap_capacity_bytes': None},
               'doop.shared_i32_values': {'distinct': len(int_values), 'min': min(int_values), 'max': max(int_values),
               'basis': 'measured union of every i32 input column; optional shared dense remap, not per-set ranks',
               'dense_mapping_bytes_estimate': 8 * len(int_values), 'new_gpu_mapping_bytes': 8 * len(int_values)}}
    component_domains, column_component = semantic_domains(con, schemas, rule_text)
    structures, relation_stats, order_cache = [], {}, {}
    for item in inventory:
        name, order, kind = item['relation'], item['order'], item['kind']
        arity = len(order)
        if kind == 'DELTA':
            # Delta-first has no separately maintained delta layouts for this program.
            raise ValueError('Unexpected DELTA requires round-specific rows, not final full rows')
        where = ''
        if kind == 'NEG' and name == 'method_modifier':
            where = f' WHERE c0={codes["abstract"]}'
        columns = ','.join(f'c{column}' for column in order)
        cache_key = (name, tuple(order), where)
        if cache_key not in order_cache:
            values = con.execute(f'SELECT DISTINCT {columns} FROM "{name}"{where} ORDER BY {columns}').fetchnumpy()
            a = np.column_stack(list(values.values())).astype(np.int64, copy=False)
            n = len(a)
            stats = []
            for i, original in enumerate(order):
                col = a[:, i]
                distinct = int(len(np.unique(col)))
                low, high = (int(col.min()), int(col.max())) if n else (None, None)
                span = high - low + 1 if n else 0
                domain = 'doop.process_global_IStr' if schemas[name][original] == 'IStr' else 'doop.shared_i32_values'
                stats.append({'column': f'c{original}', 'logical_type': schemas[name][original],
                              'min': low, 'max': high, 'distinct': distinct, 'span': span,
                              'dense': span <= 32 * distinct, 'domain': domain,
                              'domain_distinct': domains[domain]['distinct'],
                              'domain_occupancy': distinct / domains[domain]['distinct'],
                              'semantic_domain': column_component[(name, original)],
                              'semantic_domain_distinct': component_domains[column_component[(name, original)]]['distinct'],
                              'semantic_domain_occupancy': distinct / component_domains[column_component[(name, original)]]['distinct'],
                              'basis': 'measured exact complete tuples; IStr min/max are shared interner codes, not string order'})
            # Sign-bias gives canonical u32 integer codes, preserving signed order.
            encoded = a.copy()
            for i, original in enumerate(order):
                if schemas[name][original] == 'i32':
                    encoded[:, i] += 1 << 31
            raw = bitmap(math.prod(s['span'] for s in stats), n, 8 + 12 * arity)
            dense = bitmap(math.prod(s['domain_distinct'] for s in stats), n)
            dense['mapping_cost_scope'] = 'shared once per workload, domains above; no private per-relation dictionary'
            semantic = bitmap(math.prod(s['semantic_domain_distinct'] for s in stats), n)
            semantic['mapping_domains'] = sorted(set(s['semantic_domain'] for s in stats))
            semantic['mapping_cost_scope'] = 'optional shared component maps charged once in semantic_remap_totals; excluded from this individual bitmap byte count'
            plain = n * arity * 4 + 8
            candidates = {'plain_u32': {'bytes': plain, 'count_bytes': 8, 'basis': 'estimated: exact rows * identity arity * 4 plus u64 count; existing shared symbol codes and signed-i32 sign bias'},
                          'for1024_fences': for_cost(encoded), 'bitmap_raw_span': raw,
                          'bitmap_shared_dense_product': dense, 'bitmap_shared_semantic_product': semantic,
                          'adaptive_binary_chunks': adaptive_binary(encoded)}
            candidates['bitmap_eligible'] = True
            candidates['bitmap_preserves'] = 'complete set identity only; all columns included, no payload/multiplicity discarded'
            candidates['best_cost_aware_bytes'] = min(plain, candidates['for1024_fences']['bytes'], raw['bytes'], dense['bytes'], candidates['adaptive_binary_chunks'].get('bytes') if arity == 2 else plain)
            order_cache[cache_key] = (n, stats, candidates)
        n, stats, candidates = order_cache[cache_key]
        width = ceildiv(arity * 32, 64) * 8
        owner = owners.get(name)
        canonical = kind == 'FULL' and item['layout_id'] == relations[name]['canonical']
        # print.rs local(\"layout\", canonical) is DeviceRadixRun state, not its
        # published view: accepted trace excludes 12 bytes/column of PackedField.
        runs = None
        if owner and kind == 'FULL':
            residual = owner['accepted_live_bytes'] - n * width - 8
            if residual >= 0 and residual % 48 == 0:
                runs = residual // 48
        if kind == 'NEG':
            # print.rs:679-688 sorts one complete projection then from_sorted
            # adopts it with replace_sorted, hence exactly one nonempty run.
            runs = int(n != 0)
        live = n * width + 12 * arity + 8 + 48 * runs if runs is not None else None
        classification = 'multi_column' if arity > 1 else ('single_dense_key' if stats[0]['dense'] else 'single_sparse_key')
        limits = ['Candidate representations are estimates, not implemented GPU formats.',
                  'Logical per-layout owner totals can overlap physical allocations through shared Arc buffers; not allocator-resident or peak bytes.']
        if runs is None:
            limits.append('Per-layout final run count/descriptor bytes unobserved; full current_bytes unknown, key-only bytes exact.')
        structures.append({'workload': 'doop-gpu', 'query': 'xalan', 'scale': str(args.facts),
            'relation': name, 'provider': 'gpu_datalog_runtime::RadixLayout', 'structure_id': f'{kind.lower()}_{item["layout_id"]}',
            'kind': 'canonical_fullset' if canonical else ('negative_keyset' if kind == 'NEG' else 'index_order'),
            'emitted_layout_id': item['layout_id'], 'order': order, 'lookup_prefix': order[:item['lookup_arity']],
            'keys': [f'c{i}' for i in order], 'payload': [], 'set_or_bag': 'set', 'identity_arity': arity,
            'rows': n, 'distinct_keys': n, 'raw_fact_rows': raw_counts.get(name), 'key_stats': stats,
            'rows_basis': evidence[name][0],
            'physical_word': 'u64' if width == 8 else ('u128' if width == 16 else f'[u64; {width // 8}]'),
            'physical_word_bytes': width, 'key_bytes': n * width, 'directory_bytes': 0, 'offset_bytes': 0,
            'offset_basis': 'implicit unit offsets; radix_bits=0 means no radix directory',
            'field_descriptor_bytes': 12 * arity, 'run_count': runs, 'run_descriptor_bytes': 48 * runs if runs is not None else None,
            'count_bytes': 8, 'current_bytes': live,
            'current_allocated_bytes': owner['accepted_bytes'] + 12 * arity if canonical and owner else None,
            'current_bytes_basis': ('modeled exact logical live bytes from complete projected tuples and source ABI; negative projection sorts once and from_sorted adopts one nonempty run (print.rs:679-688)'
                                    if kind == 'NEG' else
                                    'modeled logical live bytes from measured tuple count and source ABI; run count inferred from revision-matched canonical live trace; full layouts share batch cardinality/merge-ratio history'),
            'candidates': candidates, 'class': classification, 'evidence': evidence[name] + [str(args.layouts),
                str(args.log) + (':' + str(owner['source_line']) if owner else ''),
                'experiments/gpu/datalog/src/print.rs:43-77,521-542',
                'experiments/gpu/datalog/src/storage.rs:248-265,385-420,475-585',
                'experiments/gpu/datalog-runtime/src/radix_run.rs:430-478,2273-2290',
                'experiments/gpu/datalog-runtime/src/layout.rs:5-12'], 'limitations': limits})
        if kind == 'FULL':
            relation_stats[name] = {'rows': n, 'source': evidence[name], 'memory_trace': owner}
            if owner and owner['rows'] != n:
                raise ValueError(f'{name}: exact tuples {n} differ from GPU final count {owner["rows"]}')
    full = [s for s in structures if s['kind'] != 'negative_keyset']
    layout_trace_agreement = {}
    for name, owner in owners.items():
        members = [s for s in full if s['relation'] == name]
        if members and all(s['current_bytes'] is not None for s in members):
            modeled = sum(s['current_bytes'] for s in members)
            layout_trace_agreement[name] = modeled == owner['layout_live_bytes']
            if modeled != owner['layout_live_bytes']:
                raise ValueError(f'{name}: emitted-layout live bytes {modeled} disagree with GPU trace {owner["layout_live_bytes"]}')
    totals = {'scope': 'sum of all emitted full layouts, including canonical exactly once; logical owner footprint, not physical co-resident memory; negatives separate',
              'structures': len(full), 'key_bytes': sum(s['key_bytes'] for s in full),
              'current_bytes': sum(s['current_bytes'] for s in full) if all(s['current_bytes'] is not None for s in full) else None,
              'plain_u32_bytes': sum(s['candidates']['plain_u32']['bytes'] for s in full),
              'for1024_fences_bytes': sum(s['candidates']['for1024_fences']['bytes'] for s in full),
              'forced_raw_span_bitmap_bytes': sum(s['candidates']['bitmap_raw_span']['bytes'] for s in full),
              'forced_shared_domain_bitmap_bytes': sum(s['candidates']['bitmap_shared_dense_product']['bytes'] for s in full),
              'cost_aware_bytes': sum(s['candidates']['best_cost_aware_bytes'] for s in full)}
    unique_orders = {(s['relation'], tuple(s['order'])): s for s in full}
    metadata_bytes = sum(s['current_bytes'] - s['key_bytes'] for s in full)
    shared_key_lower = sum(s['key_bytes'] for s in unique_orders.values())
    shared_key_upper = (
        sum(s['key_bytes'] for s in full if not relations[s['relation']]['edb'])
        + sum(s['key_bytes'] for s in unique_orders.values() if relations[s['relation']]['edb']))
    negative = [s for s in structures if s['kind'] == 'negative_keyset']
    negative_keys = sum(s['key_bytes'] for s in negative)
    negative_live = sum(s['current_bytes'] for s in negative)
    physical_bounds = {
        'scope': 'immutable key buffers and their published metadata for all inventoried full layouts and both negative keysets; not allocator capacity, retained old snapshots, transient deltas, or query peak',
        'full_unique_relation_orders': len(unique_orders),
        'full_key_live_lower_bytes': shared_key_lower,
        'full_key_live_upper_bytes': shared_key_upper,
        'full_metadata_live_bytes': metadata_bytes,
        'negative_key_live_bytes': negative_keys,
        'negative_metadata_live_bytes': negative_live - negative_keys,
        'negative_total_live_bytes': negative_live,
        'inclusive_live_lower_bytes': shared_key_lower + metadata_bytes + negative_live,
        'inclusive_live_upper_bytes': shared_key_upper + metadata_bytes + negative_live,
        'basis': 'Rigorous source-derived bounds: admitted_orders builds one batch per relation/order and full_layouts clones it into same-order states. Each state merges independently, creating private merged buffers. Lower assumes maximum sharing within each order; upper counts IDB buffers independently but deduplicates EDB same-order buffers because EDB loading publishes once. Snapshot descriptors, counts and fields are distinct per layout. Both negative projections use one independently sorted run.',
        'evidence': ['experiments/gpu/datalog/src/print.rs:109-153,593-621,679-688',
                     'experiments/gpu/datalog-runtime/src/radix_run.rs:1128-1185,2179-2189']}
    inclusive_totals = {
        'scope': 'logical ownership sum across all 225 structures; compare candidate costs on this identical scope; physical alias bounds reported separately',
        'structures': len(structures), 'current_bytes': sum(s['current_bytes'] for s in structures),
        'key_bytes': sum(s['key_bytes'] for s in structures),
        'plain_u32_bytes': sum(s['candidates']['plain_u32']['bytes'] for s in structures),
        'for1024_fences_bytes': sum(s['candidates']['for1024_fences']['bytes'] for s in structures),
        'forced_raw_span_bitmap_bytes': sum(s['candidates']['bitmap_raw_span']['bytes'] for s in structures),
        'forced_shared_domain_bitmap_bytes': sum(s['candidates']['bitmap_shared_dense_product']['bytes'] for s in structures),
        'cost_aware_bytes': sum(s['candidates']['best_cost_aware_bytes'] for s in structures)}
    component_mapping_bytes = sum(domain['new_gpu_mapping_bytes_estimate'] for domain in component_domains.values())
    semantic_remap_totals = {
        'scope': 'all 225 structures; optional shared rule-component dictionaries, each paid once; logical owner sum, not alias-deduplicated physical footprint',
        'mapping_bytes': component_mapping_bytes,
        'forced_component_bitmap_data_and_rank_bytes': sum(s['candidates']['bitmap_shared_semantic_product']['bytes'] for s in structures),
        'forced_component_bitmap_with_mapping_bytes': component_mapping_bytes + sum(s['candidates']['bitmap_shared_semantic_product']['bytes'] for s in structures),
        'cost_aware_with_all_maps_bytes': component_mapping_bytes + sum(
            min(s['candidates']['best_cost_aware_bytes'], s['candidates']['bitmap_shared_semantic_product']['bytes'])
            for s in structures),
        'basis': 'feasible conservative strategy enables all shared component maps, then chooses old no-remap best or component-product bitmap per structure; map cost is never free and original no-remap totals are retained'}
    improvements = []
    for structure in structures:
        candidate = structure['candidates']['bitmap_shared_semantic_product']
        gain = structure['candidates']['best_cost_aware_bytes'] - candidate['bytes']
        if gain > 0:
            improvements.append((set(candidate['mapping_domains']), gain))
    useful_domains = sorted(set().union(*(required for required, _ in improvements)))
    best_gain, selected_domains = 0, []
    # Xalan has four useful components, so exhaustive shared-cost selection is
    # only sixteen cases and does not charge unrelated unused maps.
    for mask in range(1 << len(useful_domains)):
        selected = {domain for bit, domain in enumerate(useful_domains) if mask & (1 << bit)}
        gain = sum(gain for required, gain in improvements if required <= selected)
        gain -= sum(component_domains[domain]['new_gpu_mapping_bytes_estimate'] for domain in selected)
        if gain > best_gain:
            best_gain, selected_domains = gain, sorted(selected)
    semantic_remap_totals.update({
        'cost_aware_selected_maps_bytes': inclusive_totals['cost_aware_bytes'] - best_gain,
        'selected_mapping_domains': selected_domains,
        'selected_mapping_bytes': sum(component_domains[domain]['new_gpu_mapping_bytes_estimate'] for domain in selected_domains),
        'selection_basis': 'exact enumeration of useful component-map subsets; charge each selected map once, collect a structure saving only if all required domains are mapped',
        'useful_domain_subsets_examined': 1 << len(useful_domains)})
    result = {'workload': 'doop-gpu', 'query': 'xalan', 'commit': args.commit,
              'binary': str(args.binary), 'binary_sha256': file_sha256(args.binary),
              'facts': str(args.facts), 'fact_sha256': fact_hashes, 'domains': domains,
              'semantic_domains': component_domains, 'semantic_remap_totals': semantic_remap_totals,
              'source_sha256': {str(path.relative_to(args.repo)): hashlib.sha256(path.read_bytes()).hexdigest()
                                for path in [macro_path, shared_path,
                                    args.repo / 'experiments/gpu/datalog/src/storage.rs',
                                    args.repo / 'experiments/gpu/datalog/src/print.rs',
                                    args.repo / 'experiments/gpu/datalog-runtime/src/radix_run.rs']},
              'coverage': {'relations': len(relations), 'edb_relations': sum(r['edb'] for r in relations.values()),
                           'full_layouts': len(full), 'canonical_sets': sum(s['kind'] == 'canonical_fullset' for s in full),
                           'delta_layouts': sum(i['kind'] == 'DELTA' for i in inventory), 'negative_keysets': sum(i['kind'] == 'NEG' for i in inventory),
                           'relations_with_exact_tuples': len(loaded), 'unknown_relation_rows': []},
              'measurement_consistency': {'all_final_relation_counts_match_exact_tuples': True,
                                          'per_relation_layout_live_bytes_match': layout_trace_agreement},
              'methods': {'layout_inventory': 'host compile of exact mini_engine source through stratify, incremental DeltaFirst, storage; device_storage preserves layout coverage/order by checked restoration',
                          'input_identity': 'Facts::load deduplicates full rows; raw_fact_rows retains file occurrences. All emitted DOOP structures are sets, not bags.',
                          'idb': '26 full GPU outputs; remaining acyclic IDBs evaluated as exact SQL projections/unions/joins of real facts using authored rules, checked against all available final GPU owner counts.',
                          'symbols': 'Replay Method first, computed descriptors in BTreeMap/BTreeSet code order, then Facts load order; constants appended only if absent. No private per-set ranking.',
                          'constants_absent_after_load': absent_constants,
                          'integer_dense_mapping': 'Optional shared integer-value rank map requires 8*distinct bytes once; plain/FOR use signed bias instead and require no mapping.',
                          'mapping': 'Existing process-global IStr string dictionary is host-resident sunk/base mapping; GPU operates only on codes and has no string dictionary upload. Exact string bytes and estimated reverse-vector live bytes reported separately; hash capacity and allocator overhead unknown.',
                          'bitmap': 'Raw-span product uses observed per-column min-relative integer spans and 4-byte base/8-byte span per coordinate; dense product uses the entire shared column universe per coordinate, never relation-private distinct ranks. Full tuple identity preserved; rank included in main bytes for position/count reads, membership-only bytes also recorded. Binary adaptive operates within each first-key group and shared second-column 2^16 chunks.',
                          'bag': 'No bag structure in this DOOP plan. Bitmap alone would not preserve duplicate occurrences or attached payload in a bag.',
                          'memory': 'Only immutable final per-structure data+descriptor ownership is compared. Candidate scratch, input host vectors, temporary deltas, string dictionaries and allocator cache are not silently mixed into those totals; relation aggregate allocated traces retained separately.'},
              'relation_stats': relation_stats, 'totals': totals,
              'inclusive_totals': inclusive_totals, 'physical_live_bounds': physical_bounds,
              'structures': structures}
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'coverage': result['coverage'], 'totals': totals, 'absent_constants': absent_constants}, indent=2))


if __name__ == '__main__':
    main()
