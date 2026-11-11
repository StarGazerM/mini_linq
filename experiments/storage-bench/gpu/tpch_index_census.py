#!/usr/bin/env python3
"""Census recorded advised Structures against the exported TPC-H binary columns.

This is an offline analysis, not an executor or an engine change. DuckDB evaluates
recorded relational operators; unsupported expressions remain explicitly unknown.
Run with --plans /path/to/out/plans.json --output /tmp/tpch-index-census.json.
"""
import argparse
import datetime
import hashlib
import json
import pathlib
import re

import duckdb
import numpy as np


def names(text):
    return [x.strip() for x in text.split(',') if x.strip()]


def ident(x):
    return '"' + x.replace('"', '""') + '"'


def balanced(text, pos, left='{', right='}'):
    depth = 0
    for end in range(pos, len(text)):
        depth += (text[end] == left) - (text[end] == right)
        if depth == 0:
            return text[pos + 1:end], end + 1
    raise ValueError('unbalanced recorded syntax')


def width(low, high):
    return 0 if low is None else (int(high) - int(low)).bit_length()


def encoded_capacity(rows, bits, gather=False):
    blocks = (rows + 1023) // 1024
    if gather:
        return max(1, blocks) * 32 + max(1, blocks * bits * 16) * 8
    return blocks * (32 + bits * 128) + 8


class Census:
    def __init__(self, root, threads, generated=''):
        self.root = pathlib.Path(root)
        self.db = duckdb.connect()
        self.db.execute(f"SET threads={threads}")
        self.db.execute("SET memory_limit='4GB'")
        self.db.execute("SET preserve_insertion_order=false")
        self.base_stats = {}
        self.arrays = {}
        self.dicts = {}
        self.counter = 0
        self.generated = generated

    def load(self, sf):
        self.sf = sf
        self.directory = self.root / f'sf{sf}'
        self.manifest = json.loads((self.directory / 'manifest.json').read_text())
        self.columns = {}
        for table, rel in self.manifest['relations'].items():
            data = {}
            for col in rel['columns']:
                key = table + '.' + col['name']
                self.columns[key] = col
                dtype = {'i64': '<i8', 'i32': '<i4', 'u32': '<u4', 'u64': '<u8'}[col['type']]
                data[col['name']] = np.memmap(self.directory / col['file'], mode='r', dtype=dtype)
            self.db.register('base_' + table, data)
            self.arrays[table] = data
        self.base_stats = {}
        self.dicts = {}

    def dictionary(self, key):
        if key not in self.dicts:
            self.dicts[key] = (self.directory / self.columns[key]['dictionary']).read_text().splitlines()
        return self.dicts[key]

    def stats(self, view, cols):
        expr = ['count(*)']
        for c in cols:
            z = ident(c)
            expr.extend([f'min({z})', f'max({z})', f'count(distinct {z})', f'count(*)-count({z})'])
        row = self.db.execute(f'SELECT {",".join(expr)} FROM {ident(view)}').fetchone()
        result = {}
        for i, c in enumerate(cols):
            low, high, distinct, nulls = row[1 + 4*i:5 + 4*i]
            result[c] = dict(min=low, max=high, distinct=distinct, nulls=nulls,
                             span=None if low is None else int(high)-int(low)+1)
        return row[0], result

    def universe(self, origin):
        if origin not in self.columns:
            return None
        # The full exported column is the shared universe, never the filtered set.
        if origin not in self.base_stats:
            table, col = origin.split('.')
            n, values = self.stats('base_' + table, [col])
            s = values[col]
            s.update(rows=n, source=str(self.directory / self.columns[origin]['file']),
                     physical_type=self.columns[origin]['type'])
            if 'dictionary_entries' in self.columns[origin]:
                s['domain_size'] = self.columns[origin]['dictionary_entries']
                s['mapping_bytes'] = 0
                s['mapping_basis'] = 'existing shared manifest dictionary codes; dictionary text is base-data storage, not new candidate map'
            else:
                s['domain_size'] = s['distinct']
                s['mapping_bytes'] = 0 if s['span'] == s['distinct'] else s['distinct'] * 8
                s['mapping_basis'] = 'min-relative direct code' if s['mapping_bytes'] == 0 else 'estimated sorted i64 code-to-value universe; forward mapping by binary search; shared once per universe'
            self.base_stats[origin] = s
        return self.base_stats[origin]

    def shared_origin(self, origin):
        # Equality-connected TPC-H primary/foreign-key columns use one codebook.
        return {
            'lineitem.l_orderkey': 'orders.o_orderkey',
            'lineitem.l_partkey': 'part.p_partkey',
            'partsupp.ps_partkey': 'part.p_partkey',
            'lineitem.l_suppkey': 'supplier.s_suppkey',
            'partsupp.ps_suppkey': 'supplier.s_suppkey',
            'orders.o_custkey': 'customer.c_custkey',
            'customer.c_nationkey': 'nation.n_nationkey',
            'supplier.s_nationkey': 'nation.n_nationkey',
            'nation.n_regionkey': 'region.r_regionkey',
        }.get(origin, origin)

    def view(self, sql):
        self.counter += 1
        name = f'v{self.counter}'
        self.db.execute(f'CREATE TEMP VIEW {name} AS {sql}')
        return name

    def input_relations(self, q):
        src = q['stage_source']
        domains = dict((d, t + '.' + c) for d, t, c in re.findall(r'domain (\w+) = dictionary \((\w+) \. (\w+)\)', src))
        inputs = {}
        for m in re.finditer(r'input (\w+) \(([^;]*?)\) = table \((\w+)\)', src):
            name, fields, table = m.groups()
            columns = [f.strip().split(' : ')[0] for f in fields.split(' , ')]
            inputs[name] = (self.view('SELECT ' + ','.join(map(ident, columns)) + ' FROM base_' + table), {c: table + '.' + c for c in columns})
        predicates = {
            'BrassTypes': lambda v: v.endswith('BRASS'),
            'ForestNames': lambda v: v.startswith('forest'),
            'GreenNames': lambda v: 'green' in v,
            'AllowedKinds': lambda v: not v.startswith('MEDIUM POLISHED'),
            'BadComments': lambda v: 'Customer' in v and 'Complaints' in v.split('Customer', 1)[1],
            'HighPriorities': lambda v: v in ('1-URGENT', '2-HIGH'),
            'SelectedComments': lambda v: not ('special' in v and 'requests' in v.split('special', 1)[1]),
            'QualifyingPhones': lambda v: v.startswith(('13','31','23','29','30','18','17')),
        }
        for m in re.finditer(r'input (\w+) \(([^;]*?)\) = dictionary \((\w+)\) where (.*?) ;', src):
            name, fields, domain, predicate = m.groups()
            origin = domains[domain]
            values = np.array([i for i, value in enumerate(self.dictionary(origin)) if predicates[name](value)], dtype=np.int64)
            col = fields.split(' : ')[0]
            alias = f'dict_{self.sf}_{q["query"]}_{name}'
            self.db.register(alias, {col: values})
            inputs[name] = (alias, {col: origin})
        if 'input QualifiedOrders ' in src:
            allowed = inputs['SelectedComments'][0]
            inputs['QualifiedOrders'] = (self.view(f'SELECT o_orderkey,o_custkey FROM base_orders SEMI JOIN {allowed} ON o_comment=code'), {'o_orderkey':'orders.o_orderkey','o_custkey':'orders.o_custkey'})
        if 'input LateLineitem ' in src:
            inputs['LateLineitem'] = (self.view('SELECT l_orderkey AS order_key FROM base_lineitem WHERE l_commitdate<l_receiptdate'), {'order_key':'lineitem.l_orderkey'})
        operands = {}
        for name, date in re.findall(r'operand (\w+) : Date = date \{ Date :: load \("([\d-]+)"\)', src):
            operands[name] = (datetime.date.fromisoformat(date)-datetime.date(1970,1,1)).days
        for name, value in re.findall(r'operand (\w+) : Decimal = rust \{ Decimal :: from_scaled \((\d+)\)', src):
            operands[name] = int(value)
        return inputs, domains, operands

    def expression(self, text, operands, captures):
        text = re.sub(r'@\s*\(', '(', text)
        text = re.sub(r'\.\s*(?:days_since_epoch|scaled)\s*\(\s*\)', '', text)
        text = re.sub(r'i128\s*::\s*from\s*\(', '(', text)
        text = re.sub(r'Some\s*\(', '(', text)
        text = re.sub(r'\[([\d ,]+)\]\s*\.\s*contains\s*\((\w+)\)', r'\2 IN (\1)', text)
        text = re.sub(r'\b\d[\d_]*\b', lambda m: m[0].replace('_', ''), text)
        text = re.sub(r'(^|[(<>=!+\-*/])\s*\*\s*(?=\w)', r'\1', text)
        for name, value in {**operands, **captures}.items():
            text = re.sub(r'\b' + re.escape(name) + r'\b', str(value), text)
        text = text.replace('&&', ' AND ').replace('||', ' OR ').replace('==', '=')
        # Predicate commas in physical syntax denote conjunction.
        if ' IN ' not in text:
            text = text.replace(' , ', ' AND ')
        return text

    def physical(self, q, stage, inputs, domains, operands):
        text = stage['physical_planned']
        relations = {}
        fields = {}
        for name, body in re.findall(r'relation (\w+) \((.*?)\) ;', text):
            cols = re.findall(r'(\w+) : ([^,]+)', body)
            fields[name] = [(c, t.strip()) for c, t in cols]
            if name in inputs:
                view, origins = inputs[name]
                # Dictionary inputs may use a stage-local field name.
                original = list(origins)
                renamed = {c: origins[old] for (c, _), old in zip(fields[name], original)}
                sql = 'SELECT ' + ','.join(f'{ident(old)} AS {ident(c)}' for (c, _), old in zip(fields[name], original)) + ' FROM ' + ident(view)
                relations[name] = (self.view(sql), renamed)
        captures = {name: int(value) for name, value in re.findall(r'let (\w+) : \w+ = (-?\d+) ;', text)}
        for name, domain, value in re.findall(r'domain (\w+) : \w+ in \[\d+\] \((\w+)\) = exact \("(.*?)"\)', text):
            dictionary = self.dictionary(domains[domain])
            # A literal absent from the shared dictionary cannot equal any row.
            captures[name] = dictionary.index(value) if value in dictionary else -1
        errors = {}
        operations = {}
        for m in re.finditer(r'\b(r\d+) = (?:let|pipe) (.*?)(?= ; (?:r\d+ =|return)| ; \}$)', text):
            name, op = m.groups()
            operations[name] = op
            try:
                if op.startswith('rename '):
                    source, body = re.match(r'rename (\w+) \{ (.*?) \}', op).groups()
                    view, origins = relations[source]
                    pairs = re.findall(r'(\w+) - > (\w+)', body)
                    lineage = {b: origins[a] for a,b in pairs}
                    sql = 'SELECT '+','.join(f'{ident(a)} AS {ident(b)}' for a,b in pairs)+' FROM '+ident(view)
                elif re.match(r'(?:range select|point select|select|guard) ', op):
                    source, expr = re.match(r'(?:range select|point select|select|guard) (r\d+) if (.*)', op).groups()
                    view, lineage = relations[source]
                    sql = f'SELECT * FROM {ident(view)} WHERE {self.expression(expr, operands, captures)}'
                elif op.startswith('bind '):
                    source, col, expr = re.match(r'bind (r\d+) with (\w+) : .*? = (.*)', op).groups()
                    view, origins = relations[source]
                    lineage = dict(origins)
                    expression = self.expression(expr, operands, captures)
                    alias = expression.strip(' ()')
                    lineage[col] = origins.get(alias)
                    sql = f'SELECT *, {expression} AS {ident(col)} FROM {ident(view)}'
                elif op.startswith(('project ', 'canon project ')):
                    source, cols = re.search(r'project (r\d+) keep \{ (.*?) \}', op).groups()
                    view, origins = relations[source]
                    cols = names(cols)
                    lineage = {c:origins[c] for c in cols}
                    sql = 'SELECT '+('DISTINCT ' if op.startswith('canon') else '')+','.join(map(ident,cols))+' FROM '+ident(view)
                elif op.startswith(('probe ', 'semi ', 'anti ')):
                    kind, keys, left, right = re.match(r'(probe|semi|anti) \w+ \[(.*?)\] (r\d+) , (r\d+)', op).groups()
                    lv, lo = relations[left]; rv, ro = relations[right]
                    keys = names(keys)
                    lineage = dict(lo)
                    if kind == 'probe':
                        lineage.update(ro)
                        selected = [f'l.{ident(c)}' for c in lo] + [f'r.{ident(c)}' for c in ro if c not in lo]
                    else:
                        selected = ['l.*']
                    cond = ' AND '.join(f'l.{ident(k)}=r.{ident(k)}' for k in keys) or 'true'
                    join = {'probe':'JOIN','semi':'SEMI JOIN','anti':'ANTI JOIN'}[kind]
                    sql = f'SELECT {",".join(selected)} FROM {ident(lv)} l {join} {ident(rv)} r ON {cond}'
                elif op.startswith('group '):
                    source, keys, body = re.match(r'group (r\d+) by \[(.*?)\] \{ (.*?) \}', op).groups()
                    view, origins = relations[source]
                    keys = names(keys)
                    lineage = {c:origins[c] for c in keys}
                    selected = list(map(ident,keys))
                    for col, fun, arg in re.findall(r'(\w+) : .*? = (\w+) \((.*?)\) ;',body):
                        lineage[col] = None
                        argument = self.expression(arg, operands, captures) if arg else '*'
                        selected.append(f'{fun}({argument}) AS {ident(col)}')
                    sql = f'SELECT {",".join(selected)} FROM {ident(view)}'+(' GROUP BY '+','.join(map(ident,keys)) if keys else '')
                elif op.startswith('union '):
                    left,right = re.match(r'union \w+ (r\d+) , (r\d+)',op).groups()
                    lv, lineage = relations[left]; rv,_ = relations[right]
                    sql = f'SELECT * FROM {ident(lv)} UNION SELECT * FROM {ident(rv)}'
                else:
                    raise ValueError('unsupported physical operator: '+op)
                relations[name] = (self.view(sql), dict(lineage))
            except (KeyError, ValueError, duckdb.Error) as exc:
                errors[name] = str(exc)
        # Publication schema is the physical answer heading, in return order.
        match = re.search(r'return (r\d+) as answer \((.*?)\)', text)
        if match and match[1] in relations:
            view, origins = relations[match[1]]
            actual = names(match[2]); declared = [c for c,_ in fields['answer']]
            sql = 'SELECT '+','.join(f'{str(captures[a]) if a in captures else ident(a)} AS {ident(b)}' for a,b in zip(actual, declared))+' FROM '+ident(view)
            output = (self.view(sql), {b:origins.get(a) for a,b in zip(actual,declared)})
        else:
            output = None
        return relations, operations, errors, output

    def structure_declarations(self, text):
        # Canon is a policy for the following rows provider, not a second allocation.
        pattern = r'\b(r\d+)\s+(?:(rows|group)\s+)?= @ (\w+) as (.*?)(?= ;| \}$)'
        for m in re.finditer(pattern,text):
            relation, role, provider, body = m.groups()
            key = re.search(r'\[(.*?)\]',body)
            payload = re.search(r'payload \[(.*?)\]',body)
            yield dict(relation=relation, role=role or 'index', provider=provider,
                       keys=names(key[1]) if key else [],
                       logical_payload=names(payload[1]) if payload else None,
                       membership=' membership' in body, declaration=m[0])

    def canonical_layout(self, q, stage, relations):
        module = f'mod {q["query"]}_sf{q["sf"]}_advised {{'
        start = self.generated.find(module)
        if start < 0:
            return None
        body, _ = balanced(self.generated, start + len(module) - 1)
        marker = f'mod stage_{stage["stage"]} {{'
        start = body.find(marker)
        if start < 0:
            return None
        body, _ = balanced(body, start + len(marker) - 1)
        call = body.find('>>::build_set(')
        if call < 0:
            return None
        begin = body.rfind('let mut __cq_key_bits', 0, call)
        text = re.sub(r'\s+', '', body[begin:call])
        fields = []
        for match in re.finditer(r'let\(__cq_pack_base_(\d+),__cq_width\)=(.*?);let__cq_pack_shift_\1=', text):
            expr = match[2]
            scalar = re.search(r'<(i64|u64|i32|u32|usize)as::gpu_datalog_runtime::Scalar>::MIN', expr)
            if scalar:
                bits = 32 if scalar[1].endswith('32') else 64
                fields.append(dict(bits=bits, basis='generated full scalar domain '+scalar[1]))
                continue
            refs = re.findall(r'__cq_sequence_host_(\w+)\.storage\(\)\.bounds\[(\d+)usize\]', expr)
            bounds = []
            for name, index in refs:
                if name not in relations or int(index) >= len(relations[name][1]):
                    return None
                view, origins = relations[name]
                column = list(origins)[int(index)]
                universe = self.universe(origins[column])
                if universe is None:
                    _, measured = self.stats(view, [column])
                    universe = measured[column]
                if universe['min'] is not None:
                    bounds.append((universe['min'], universe['max']))
            if not refs:
                return None
            bits = width(min(x[0] for x in bounds), max(x[1] for x in bounds)) if bounds else 0
            fields.append(dict(bits=bits, basis='generated union of source bounds', source_fields=refs))
        if not fields:
            return None
        bits = sum(x['bits'] for x in fields)
        return dict(key_bits=bits, key_lane_bytes=8*max(1,(bits+63)//64),
                    fields=fields, directory_bytes=0, offsets_bytes=0,
                    metadata_bytes=16, allocation_capacity=None,
                    basis='generated build_set packing expressions with measured source-column bounds; capacity not recorded')

    def measure(self, q, stage, decl, relations, operations, errors):
        name=decl['relation']; provider=decl['provider']; keys=decl['keys']
        out=dict(workload='gpu_tpch_advised',query=q['query'],scale=q['sf'],stage=stage['stage'],
                 relation=name,provider=provider,keys=keys,payload=None,set_or_bag='set' if provider=='radix_set' else 'bag',
                 identity_arity=None,rows=None,distinct_keys=None,key_stats={},current_bytes=None,
                 current_bytes_basis='unknown',candidates={},**{'class':'multi_column'},
                 evidence={'structures_declaration':decl['declaration'],'physical_definition':operations.get(name),
                           'plan_stage':stage['stage']},limitations=[])
        if name not in relations:
            out['limitations'].append('Exact relational construction unavailable: '+errors.get(name,'missing relation'))
            return out
        view, origins=relations[name]
        if provider=='radix_set':
            keys=list(origins)
        elif decl['role']=='group':
            m=re.search(r'by \[(.*?)\]',operations.get(name,'')); keys=names(m[1]) if m else []
        out['keys']=keys
        if decl['logical_payload'] is None:
            logical_payload=list(origins)
        else:
            logical_payload=decl['logical_payload']
        if decl['membership']:
            logical_payload=keys
        excluded_keys = keys if provider == 'group_map' else keys[:1]
        payload=[] if provider=='radix_set' else [c for c in logical_payload if c not in excluded_keys]
        out['payload']=payload
        out['identity_arity']=len(origins) if provider=='radix_set' else len(set(keys+payload))
        cols=list(dict.fromkeys(keys+payload))
        n, stats=self.stats(view, cols)
        distinct=self.db.execute(f'SELECT count(*) FROM (SELECT DISTINCT {",".join(map(ident,keys))} FROM {ident(view)})').fetchone()[0] if keys else int(n>0)
        out.update(rows=n,distinct_keys=distinct)
        out['evidence']['stats']='exact DuckDB evaluation of recorded Physical on read-only manifest binary columns; no sampling'
        for col,s in stats.items():
            origin=origins[col]
            domain_origin=self.shared_origin(origin)
            universe=self.universe(domain_origin)
            s['source_column']=origin
            s['source_bounds']=self.universe(origin)
            s['domain']=dict(id=f'sf{self.sf}:{domain_origin}',universe_column=domain_origin,**universe) if universe else None
            s['dense_by_span']=s['span'] is not None and s['span']<=32*s['distinct']
        out['key_stats']={c:stats[c] for c in keys}
        out['payload_stats']={c:stats[c] for c in payload}
        bag=provider not in ('radix_set','group_map') and not decl['membership'] and (n>distinct or bool(payload))
        out['set_or_bag']='bag' if bag else 'set'
        if provider == 'group_map':
            out['semantics'] = 'One row per distinct group key, with aggregate payload retained; not an occurrence-preserving bag.'
        out['class']='bag_occurrence' if bag else ('multi_column' if len(keys)!=1 else ('single_dense_key' if stats[keys[0]]['dense_by_span'] else 'single_sparse_key'))
        if decl['role'] == 'rows' and provider == 'range_provider':
            publication = stage.get('gpu_publications', {}).get(name, '')
            heading = re.search(r'storage flat_rows \[(.*?)\] width (\d+)', publication)
            out['evidence']['gpu_publication'] = publication
            out['current_bytes_basis'] = 'Rows-role range provider is a flat-row/prefix publication, not the keyed FOR RangeProvider. Runtime prefix extent unknown.'
            if heading:
                fields = re.findall(r'"([^"]+)"', heading[1])
                out['payload'] = fields
                out['identity_arity'] = len(fields)
                out['payload_stats'] = {c:stats[c] for c in fields if c in stats}
                probe = re.match(r'probe \w+ \[(.*?)\] (r\d+) , (r\d+)', operations.get(name, ''))
                physical_rows = n
                physical_view = view
                physical_columns = origins
                if probe:
                    lv, lo = relations[probe[2]]
                    rv, _ = relations[probe[3]]
                    condition = ' AND '.join(f'l.{ident(k)}=r.{ident(k)}' for k in names(probe[1])) or 'true'
                    prefix_view = self.view(f'SELECT l.* FROM {ident(lv)} l SEMI JOIN {ident(rv)} r ON {condition}')
                    physical_view = prefix_view
                    physical_columns = lo
                    physical_rows, prefix_stats = self.stats(prefix_view, [c for c in fields if c in lo])
                    out['physical_payload_stats'] = prefix_stats
                    out['payload_stats_basis'] = 'Logical expanded values; physical_payload_stats separately describes stored prefix slots.'
                    if q['query'] == 'h21_positive' and name == 'r8':
                        # Generated slots for the later expansion start at zero.
                        for column in set(fields)-set(lo):
                            prefix_stats[column] = dict(min=0 if physical_rows else None,
                                                        max=0 if physical_rows else None,
                                                        distinct=int(physical_rows>0),nulls=0,
                                                        span=int(physical_rows>0),
                                                        basis='Generated zero-initialized prefix placeholder; not the logical expanded column')
                        out['evidence']['prefix_emission_source'] = 'generated queries.rs SF1:56785-56903; allocation SF1:58668-58742'
                out['current_layout'] = dict(kind='retained_prefix' if probe else 'flat_rows',
                                             physical_rows=physical_rows,logical_expanded_rows=n,
                                             payload_lane_bytes=8,payload_columns=fields,
                                             begin_lane_bytes=4 if probe else 0,
                                             offset_lane_bytes=4 if probe else 0,
                                             metadata_bytes=20)
                if physical_rows is not None:
                    out['current_bytes'] = physical_rows*8*len(fields)+(8*physical_rows+4 if probe else 0)+20
                    out['current_bytes_basis'] = 'Source-derived allocation estimate: generated flat i64 payload rows, optional u32 begin and terminal offsets, 5-u32 metadata; prefix count measured by left semijoin, not multiplied by expansion fanout.'
                    # Store every physical occurrence, never the expanded rows.
                    # Sorting prefixes is a candidate build-time permutation:
                    # begin/range lengths follow it and offsets are recomputed.
                    projected=[]
                    for column in fields:
                        if column in physical_columns:
                            projected.append(ident(column))
                        elif q['query']=='h21_positive' and name=='r8':
                            projected.append(f'0 AS {ident(column)}')
                        else:
                            raise ValueError('unresolved physical prefix column '+column)
                    ordered=self.db.execute(
                        f'SELECT {",".join(projected)} FROM {ident(physical_view)} ORDER BY ALL'
                    ).fetchnumpy()
                    raw_eligible=True
                    frames=[]
                    packed_bytes=0
                    physical_stats=out.get('physical_payload_stats',out['payload_stats'])
                    for column in fields:
                        column_stats=physical_stats[column]
                        low,high=column_stats['min'],column_stats['max']
                        base=0 if low is None or (low>=0 and high<=2**32-1) else low
                        fits=low is None or high-base<=2**32-1
                        raw_eligible=raw_eligible and fits and column_stats['nulls']==0
                        frames.append(dict(column=column,base=base,applies=fits))
                        values=ordered[column]
                        for start in range(0,len(values),1024):
                            block=values[start:start+1024]
                            packed_bytes+=20+128*width(block.min(),block.max())
                    frame_bytes=8*sum(frame['base']!=0 for frame in frames)
                    directory_bytes=8*physical_rows+4 if probe else 0
                    retained=directory_bytes+20
                    common=dict(kind='estimate from measured physical rows; no remapping',
                                scope='physical prefix/flat payload; all occurrences retained, never expanded-row materialization',
                                applies=raw_eligible,physical_rows=physical_rows,frames=frames,
                                frame_metadata_bytes=frame_bytes,mapping_bytes_separate=0,
                                retained_begin_and_offsets_bytes=directory_bytes,
                                retained_metadata_bytes=20,retained_payload_bytes=0)
                    out['candidates']['raw_u32_codes']=dict(common,
                        bytes=4*physical_rows*len(fields)+frame_bytes+retained if raw_eligible else None,
                        payload_code_bytes=4*physical_rows*len(fields) if raw_eligible else None)
                    out['candidates']['raw_for1024_fences']=dict(common,
                        bytes=packed_bytes+frame_bytes+retained if raw_eligible else None,
                        payload_code_bytes=packed_bytes if raw_eligible else None,
                        block_rows=1024,header_bytes=16,fence_bytes=4,
                        ordering='lexicographic physical prefix tuple, with aligned begin/range-length permutation and recomputed offsets')
            out['limitations'].append('Bitmap-only replacement cannot preserve the expanded bag. Raw u32/FOR candidates encode physical prefix/flat payload while retaining all occurrences, begin/offset arrays and 20-byte metadata; no dense interning is assumed.')
            return out
        if provider not in ('range_provider', 'sorted_run', 'group_map'):
            out['limitations'].append('Canonical allocation capacity is not observed; logical live bytes are separate from retained allocation capacity.')
            if provider=='radix_set':
                out['current_bytes_basis']='PackedRow u64 lanes ceil(key_bits/64), zero directory, implicit unit offsets, 16-byte OutputCount; allocation retains pre-dedup capacity, not distinct row count. key_bits and capacity unknown.'
                out['current_layout'] = self.canonical_layout(q, stage, relations)
                if out['current_layout'] is not None:
                    out['current_layout']['logical_live_key_bytes'] = n * out['current_layout']['key_lane_bytes']
                    out['current_live_bytes'] = n * out['current_layout']['key_lane_bytes'] + out['current_layout']['metadata_bytes']
                    out['current_live_bytes_basis'] = 'Measured distinct rows times generated packed word bytes plus 16-byte device OutputCount; no claim about retained pre-dedup capacity.'
                    out['current_layout']['host_schema_descriptor_bytes_separate'] = 16*len(keys)
                    out['current_bytes_basis'] = 'Known generated packed widths and live key bytes; retained pre-dedup allocation capacity remains unknown, so no current total is invented.'
            if provider != 'radix_set':
                return out
        # RangeProvider preserves original input occurrence count even for membership.
        # Its leading key is reconstructed from offsets, not persistently stored.
        payload_bytes=0
        payload_parts=[]
        for col in payload:
            s=stats[col]; domain=s['source_bounds']
            lo,hi=(domain['min'],domain['max']) if domain else (s['min'],s['max'])
            bits=width(lo,hi)
            cost=encoded_capacity(n,bits,True)
            payload_bytes+=cost
            payload_parts.append(dict(column=col,allocated_slot_bits=bits,bytes=cost,bounds_basis='full source-column bounds' if domain else 'measured selected bounds; runtime may enclose more values'))
        key_domain=stats[keys[0]]['source_bounds'] if keys else None
        span=(key_domain['span'] if key_domain else stats[keys[0]]['span']) if keys else 0
        entries=(span+1) if keys and span is not None else 0
        directory=encoded_capacity(entries,n.bit_length())
        views=40*len(payload)
        current=directory+payload_bytes+views if provider == 'range_provider' else None
        if current is not None:
            out['current_bytes']=current
            out['current_bytes_basis']='source-derived allocated capacity estimate, RangeProvider::bytes scope: FOR global-width reserved slots + 32-byte headers + offset extent + 40-byte payload views; excludes shared count-owner and transients'
            out['current_layout']=dict(directory_entries=entries,directory_bytes=directory,offset_width_bits=n.bit_length(),payload_bytes=payload_bytes,payload_columns=payload_parts,payload_view_bytes=views,stored_key_bytes=0,sort_temporaries_included=False)
        if provider == 'sorted_run':
            # The recorded host build borrows tuple fields from MinimumCost.
            # Rows::columns yields references; NoDense is SortedRun's default.
            payload_bytes = 8*len(payload)*n
            views = 0
            offset_bytes = (4 if n <= 2**32-1 else 8)*(distinct+1)
            key_bytes = 8*len(keys)*distinct
            current = key_bytes+offset_bytes+payload_bytes
            out['current_bytes'] = current
            out['current_bytes_basis'] = 'Source-derived host SortedRun borrowed-reference arrays: 8-byte key/payload references, narrow u32 occurrence offsets, NoDense binary search; borrowed backing relation and host object headers excluded.'
            out['current_layout'] = dict(stored_key_bytes=key_bytes,key_lane_bytes=8,
                                         payload_bytes=payload_bytes,payload_lane_bytes=8,
                                         offset_bytes=offset_bytes,offset_lane_bytes=4 if n <= 2**32-1 else 8,
                                         directory_bytes=0)
            out['evidence']['host_layout_sources'] = [
                'generated queries.rs:5476-5495',
                'runtime/cq-storage/src/rows.rs:21-29',
                'runtime/cq-storage/src/sorted_run.rs:20-33,103-123,256-259']
        if provider == 'group_map':
            payload_bytes = 9*n*len(payload)
            # Keep validity bytes for every key as well as every aggregate.
            views = n*len(keys)+16
            current = 9*n*out['identity_arity']+16
            out['current_bytes'] = current
            out['current_bytes_basis'] = 'Source-derived SegmentedGroup publication allocation: groups*width i64 values, groups*width u8 validity, one 16-byte OutputCount; sorting scratch and shared status owners excluded.'
            out['current_layout'] = dict(kind='grouped_output',value_lane_bytes=8,
                                         validity_lane_bytes=1,key_value_bytes=8*n*len(keys),
                                         aggregate_value_bytes=8*n*len(payload),
                                         validity_bytes=n*out['identity_arity'],
                                         metadata_bytes=16,directory_bytes=0,offset_bytes=0)
            out['evidence']['group_layout_source'] = 'experiments/gpu/sequence-runtime/src/group.rs:344-346,472-486,517-529'
        if provider == 'radix_set':
            views = 16  # Preserve the resident OutputCount in every candidate.
        retained=payload_bytes+views
        maps={stats[c]['domain']['id']:stats[c]['domain']['mapping_bytes'] for c in keys if stats[c]['domain']}
        map_cost=sum(maps.values())
        unknown_domains=any(stats[c]['domain'] is None for c in keys)
        # Key list + per-key occurrence endpoints retains payload and bag multiplicity.
        offsets=4*(distinct+1) if bag and n > distinct else 0
        needs_rank = bag or bool(payload) or provider == 'radix_set'
        codes=4*len(keys)*distinct+offsets+retained
        out['candidates']['u32_canonical_codes']=dict(bytes=codes if not unknown_domains else None,kind='estimate',key_bytes=4*len(keys)*distinct,occurrence_offsets_bytes=offsets,retained_payload_bytes=retained,mapping_bytes_separate=None if unknown_domains else map_cost,applies=None if unknown_domains else all(stats[c]['domain']['domain_size']<=2**32 for c in keys),scope='distinct key tuples plus occurrence offsets and unchanged current payload')
        if not keys:
            out['limitations'].append('Keyless rows publication has no membership bitmap or sorted key candidate.')
            out['candidates']={}
            return out
        # Exact sorted distinct keys; vector widths measured from this ordering.
        ordered=self.db.execute(f'SELECT DISTINCT {",".join(map(ident,keys))} FROM {ident(view)} ORDER BY {",".join(map(ident,keys))}').fetchnumpy()
        raw_frames=[]
        raw_for_bytes=0
        raw_eligible=True
        for col in keys:
            source=stats[col]['domain'] or stats[col]
            low,high=source['min'],source['max']
            base=0 if low is None or (low>=0 and high<=2**32-1) else low
            fits=low is None or high-base<=2**32-1
            raw_eligible=raw_eligible and fits and stats[col]['nulls']==0
            raw_frames.append(dict(column=col,base=base,applies=fits,
                                   basis='raw u32 value' if base==0 else 'exact min-relative frame; no dense remapping'))
            values=ordered[col]
            for start in range(0,len(values),1024):
                block=values[start:start+1024]
                raw_for_bytes+=20+128*width(block.min(),block.max())
        frame_bytes=8*sum(frame['base']!=0 for frame in raw_frames)
        raw_common=dict(kind='estimate from measured raw keys; no remapping',applies=raw_eligible,
                        frames=raw_frames,frame_metadata_bytes=frame_bytes,
                        mapping_bytes_separate=0,occurrence_offsets_bytes=offsets,
                        retained_payload_bytes=retained)
        out['candidates']['raw_u32_codes']=dict(raw_common,
            bytes=4*len(keys)*distinct+frame_bytes+offsets+retained if raw_eligible else None,
            key_bytes=4*len(keys)*distinct if raw_eligible else None)
        out['candidates']['raw_for1024_fences']=dict(raw_common,
            bytes=raw_for_bytes+frame_bytes+offsets+retained if raw_eligible else None,
            key_bytes=raw_for_bytes if raw_eligible else None,
            block_rows=1024,header_bytes=16,fence_bytes=4)
        for_bytes=0
        for col in keys:
            values=ordered[col]
            domain=stats[col]['domain']
            if domain is None:
                continue
            origin=domain['universe_column']
            if self.columns[origin].get('encoding') != 'dictionary':
                if domain['span'] == domain['distinct']:
                    values=values-domain['min']
                else:
                    table, source=origin.split('.')
                    universe=self.db.execute(f'SELECT DISTINCT {ident(source)} FROM base_{table} ORDER BY {ident(source)}').fetchnumpy()[source]
                    values=np.searchsorted(universe,values)
                    del universe
            for start in range(0,len(values),1024):
                block=values[start:start+1024]
                bits=width(block.min(),block.max()) if len(block) else 0
                for_bytes+=20+128*bits
        out['candidates']['for1024_fences']=dict(bytes=None if unknown_domains else for_bytes+offsets+retained,key_bytes=None if unknown_domains else for_bytes,occurrence_offsets_bytes=offsets,retained_payload_bytes=retained,kind='estimate from measured sorted shared-code block min/max',block_rows=1024,header_bytes=16,fence_bytes=4,mapping_bytes_separate=None if unknown_domains else map_cost,encoding='shared full source-column universe codes; unknown derived universes not privately interned')
        if len(keys)==1:
            s=stats[keys[0]]; u=s['domain']; rawspan=s['span'] or 0
            # rank gives key position; endpoints separately preserve arbitrary run lengths.
            rank=4*((rawspan+511)//512+1) if needs_rank else 0
            raw=(rawspan+7)//8+rank+offsets+retained
            dense_size=u['domain_size'] if u else None
            drank=4*((dense_size+511)//512+1) if needs_rank and dense_size is not None else 0
            dense=(dense_size+7)//8+drank+offsets+retained if dense_size is not None else None
            out['candidates']['bitmap']=dict(raw_span_bytes=raw,raw_bitmap_bytes=(rawspan+7)//8,raw_span=rawspan,rank_bytes=rank,dense_domain_bytes=dense,dense_domain_size=dense_size,dense_rank_bytes=drank,mapping_bytes_separate=u['mapping_bytes'] if u else None,occurrence_offsets_bytes=offsets,retained_payload_bytes=retained,occupancy_raw=distinct/rawspan if rawspan else 0,occupancy_dense=distinct/dense_size if dense_size else None,bitmap_alone_preserves_relation=not bag and not payload,kind='estimate',keep_current_fallback_bytes=min(current,raw) if current is not None else None,raw_saves_bytes=current-raw if current is not None else None)
        elif len(keys)==2:
            a,b=ordered[keys[0]],ordered[keys[1]]
            # Per-first-key 2^16 chunks of raw second-key min-relative codes.
            bbase=int(b.min()) if len(b) else 0
            chunks={}
            for start in range(0,len(a),1000000):
                aa=a[start:start+1000000]; bb=(b[start:start+1000000]-bbase)//65536
                pairs=np.rec.fromarrays([aa,bb],names='first,chunk')
                unique,counts=np.unique(pairs,return_counts=True)
                for pair,count in zip(unique,counts):
                    k=(int(pair[0]),int(pair[1]));chunks[k]=chunks.get(k,0)+int(count)
            arrays=sum(2*c for c in chunks.values() if 2*c<=8192)
            bitmaps=sum(8192 for c in chunks.values() if 2*c>8192)
            first_count=stats[keys[0]]['distinct']
            chunk_dir=16*len(chunks)+8*first_count+4
            brank=sum(4*129 for c in chunks.values() if 2*c>8192) if needs_rank else 0
            adaptive=arrays+bitmaps+chunk_dir+brank+offsets+retained
            out['candidates']['binary_adaptive_chunks']=dict(bytes=adaptive,array_bytes=arrays,bitmap_bytes=bitmaps,chunk_directory_bytes=chunk_dir,rank_bytes=brank,chunks=len(chunks),array_chunks=sum(2*c<=8192 for c in chunks.values()),bitmap_chunks=sum(2*c>8192 for c in chunks.values()),first_keys=first_count,chunk_bits=16,chunk_header_bytes=16,first_key_directory_entry_bytes=8,occurrence_offsets_bytes=offsets,retained_payload_bytes=retained,mapping_bytes_separate=0,kind='estimate from measured distinct binary keys; raw min-relative second codes',keep_current_fallback_bytes=min(current,adaptive) if current is not None else None)
        if len(keys) > 1:
            raw_domain = 1
            dense_domain = 1
            for col in keys:
                raw_domain *= stats[col]['span'] or 0
                if stats[col]['domain'] is not None:
                    dense_domain *= stats[col]['domain']['domain_size']
            raw_rank = 8*((raw_domain+511)//512+1) if needs_rank else 0
            dense_rank = 8*((dense_domain+511)//512+1) if needs_rank else 0
            out['candidates']['cartesian_bitmap'] = dict(
                raw_span_bytes=(raw_domain+7)//8+raw_rank+offsets+retained,
                raw_domain_size=raw_domain, raw_rank_bytes=raw_rank,
                dense_domain_bytes=None if unknown_domains else (dense_domain+7)//8+dense_rank+offsets+retained,
                dense_domain_size=None if unknown_domains else dense_domain,
                dense_rank_bytes=None if unknown_domains else dense_rank,
                mapping_bytes_separate=None if unknown_domains else map_cost,
                occurrence_offsets_bytes=offsets,retained_payload_bytes=retained,
                occupancy_raw=distinct/raw_domain if raw_domain else 0,
                occupancy_dense=distinct/dense_domain if dense_domain and not unknown_domains else None,
                kind='estimate; Cartesian product of measured raw spans or shared full-column domains, never tuple-private interning')
        out['limitations'].append('Candidate byte estimates are layouts, not implemented codecs; shared domain mapping charged separately. Host objects/allocator overhead and shared count owners excluded on both sides.')
        return out

    def query(self,q):
        inputs,domains,operands=self.input_relations(q)
        publications={int(stage):name for name,stage in re.findall(r'publish (\w+) = s(\d+) \w+ ;',q['stage_source'])}
        structures=[]; stage_notes=[]
        for stage in q['stages']:
            rels,ops,errors,output=self.physical(q,stage,inputs,domains,operands)
            if output and stage['stage'] in publications:
                inputs[publications[stage['stage']]]=output
            decls=list(self.structure_declarations(stage['structures_planned']))
            for decl in decls:
                try:
                    structures.append(self.measure(q,stage,decl,rels,ops,errors))
                except (duckdb.Error,ValueError,KeyError,TypeError) as exc:
                    structures.append(dict(workload='gpu_tpch_advised',query=q['query'],scale=q['sf'],stage=stage['stage'],relation=decl['relation'],provider=decl['provider'],keys=decl['keys'],payload=decl['logical_payload'],set_or_bag='unknown',identity_arity=None,rows=None,distinct_keys=None,key_stats={},current_bytes=None,current_bytes_basis='unknown',candidates={},**{'class':'multi_column'},evidence={'structures_declaration':decl['declaration']},limitations=[str(exc)]))
            stage_notes.append(dict(stage=stage['stage'],structures=len(decls),physical_errors=errors,
                                    structures_planned=stage['structures_planned'],
                                    physical_planned=stage['physical_planned']))
        for structure in structures:
            if structure['current_bytes'] is not None:
                structure['current_live_bytes'] = structure['current_bytes']
                structure['current_live_bytes_basis'] = 'Published representation footprint; retains current FOR block padding, validity and descriptors. Not logical scalar-width compression.'
        known=[x for x in structures if x['current_bytes'] is not None]
        live=[x for x in structures if x.get('current_live_bytes') is not None]
        comparison_scopes={}
        for basis,field,scope in (('allocated','current_bytes',known),('live','current_live_bytes',live)):
            comparisons={}
            for method in ('raw_u32_codes','raw_for1024_fences','u32_canonical_codes','for1024_fences','bitmap','binary_adaptive_chunks','cartesian_bitmap'):
                eligible=[]
                shared_maps={}
                fallback_maps={}
                for structure in scope:
                    candidate=structure['candidates'].get(method,{})
                    cost=candidate.get('raw_span_bytes' if method in ('bitmap','cartesian_bitmap') else 'bytes')
                    if cost is not None:
                        eligible.append((structure[field],cost))
                        if method in ('u32_canonical_codes','for1024_fences'):
                            for stats in structure['key_stats'].values():
                                domain=stats['domain']
                                if domain is not None:
                                    shared_maps[domain['id']]=domain['mapping_bytes']
                                    if cost < structure[field]:
                                        fallback_maps[domain['id']]=domain['mapping_bytes']
                comparisons[method]=dict(structures=len(eligible),current_same_scope_bytes=sum(a for a,b in eligible),
                                         forced_candidate_bytes=sum(b for a,b in eligible),
                                         keep_current_fallback_bytes=sum(min(a,b) for a,b in eligible),
                                         mapping_bytes_included=False,
                                         shared_mapping_bytes_separate=sum(shared_maps.values()),
                                         forced_candidate_with_mapping_bytes=sum(b for a,b in eligible)+sum(shared_maps.values()),
                                         fallback_with_mapping_bytes=sum(min(a,b) for a,b in eligible)+sum(fallback_maps.values()))
            comparison_scopes[basis]=comparisons
        return dict(workload='gpu_tpch_advised',query=q['query'],scale=q['sf'],plan_status=q['status'],
                    stage_source=q['stage_source'],structures=structures,stage_notes=stage_notes,
                    comparisons=comparison_scopes['allocated'],live_comparisons=comparison_scopes['live'],
                    totals=dict(structures=len(structures),
                                measured_row_structures=sum(x['rows'] is not None for x in structures),
                                known_current_bytes=sum(x['current_bytes'] for x in known),
                                unknown_current_structures=len(structures)-len(known),
                                known_current_live_bytes=sum(x['current_live_bytes'] for x in live),
                                unknown_current_live_structures=len(structures)-len(live),
                                basis='Per-query sums, not suite co-residency. Live canonical storage includes distinct packed keys and device metadata but does not invent retained pre-dedup capacity. Host schema descriptors remain separate.'))


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plans',required=True,type=pathlib.Path)
    parser.add_argument('--data-root',default='/data/tpch-gpu')
    parser.add_argument('--output',required=True,type=pathlib.Path)
    parser.add_argument('--threads',type=int,default=4)
    parser.add_argument('--query',action='append')
    parser.add_argument('--scale',type=int,action='append')
    args=parser.parse_args()
    plans=json.loads(args.plans.read_text())
    result=dict(workload='gpu_tpch_advised',date='2026-11-11',plan_source=str(args.plans),plan_sha256=hashlib.sha256(args.plans.read_bytes()).hexdigest(),methods={'scope':'Every index, rows/canonical publication and group declared in advised Structures; scan-only base inputs excluded. Canon policy and rows provider counted once. Per-query totals only.', 'stats':'Exact CPU relational evaluation over manifest binary files. Unknown means not successfully reconstructed; no synthetic cardinalities.', 'current':'RangeProvider::bytes allocated-capacity estimate. Canonical packed layouts and group providers unknown unless runtime allocation facts are available.', 'candidates':'FOR1024 16-byte header plus u32 fence per block; bitmap rank every 512 positions; binary 65536 chunks choose uint16 array vs 8192-byte bitmap. Payload and bag occurrence endpoints retained.', 'mapping':'Full exported source-column universe shared across structures, never private filtered-set interning. Sparse numeric dense maps separately charged as sorted i64 value list. Existing dictionary codes require no new map.', 'sources':['experiments/gpu/sequence-runtime/src/lib.rs:872-923,1394-1426','experiments/gpu/device-columns/src/lib.rs:392-431,477-536,682-685','experiments/gpu/device-columns/src/device.rs:39-59','experiments/gpu/datalog-runtime/src/radix_run.rs:1502-1525,1704-1767']},queries=[])
    result['methods']['mapping'] = 'Equality-connected primary/foreign key columns share the full exported primary-key universe; other columns share their full exported source-column universe. Sparse numeric maps are separately charged sorted i64 value lists, once per shared domain per query; no private filtered-set interning.'
    result['methods']['current'] = 'Separate allocated and live scopes. RangeProvider reserves FOR slot capacity; SegmentedGroup retains 9*rows*arity+16 bytes. Canonical live scope uses generated packed word width*distinct rows+16-byte device metadata, with host schema descriptor bytes listed separately; retained pre-dedup capacities remain unknown.'
    result['methods']['sources'].extend(['experiments/gpu/sequence-runtime/src/group.rs:344-346,472-486,517-529', 'experiments/gpu/sequence-runtime/src/canon.rs:40-45'])
    generated = args.plans.with_name('queries.rs')
    engine=Census(args.data_root,args.threads,generated.read_text() if generated.exists() else '')
    for sf in sorted({q['sf'] for q in plans}):
        if args.scale and sf not in args.scale: continue
        engine.load(sf)
        for q in plans:
            if q['sf']!=sf or q['mode']!='advised' or (args.query and q['query'] not in args.query): continue
            print(f'sf{sf} {q["query"]}',flush=True)
            result['queries'].append(engine.query(q))
            args.output.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({'queries':len(result['queries']),'structures':sum(len(q['structures']) for q in result['queries']),'measured':sum(q['totals']['measured_row_structures'] for q in result['queries'])}),flush=True)


if __name__=='__main__':
    main()
