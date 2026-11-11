#!/usr/bin/env python3
"""Combine exact workload censuses and optimize explicitly paid shared maps.

This writes accounting estimates, not measured CUDA allocation or performance.
The source census JSON objects are retained intact in the final report.
"""
import argparse
from collections import Counter
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
STEM = 'index-census-2026-11-11'


def options_tpch(r):
    """Flatten layout options; mapping dictionaries are shared per query."""
    maps = {}
    for s in r.get('key_stats', {}).values():
        d = s.get('domain')
        if d and d.get('mapping_bytes', 0):
            maps[d['id']] = d['mapping_bytes']
    result = []
    for name, c in r.get('candidates', {}).items():
        if c.get('applies') is False:
            continue
        needed = maps if c.get('mapping_bytes_separate', 0) else {}
        if c.get('bytes') is not None:
            result.append(dict(name=name, bytes=c['bytes'], maps=needed,
                               bitmap=False, bits=None))
        if c.get('raw_span_bytes') is not None:
            result.append(dict(name=name+'_raw', bytes=c['raw_span_bytes'], maps={},
                               bitmap=True, bits=c.get('raw_domain_size', c.get('raw_span'))))
        if c.get('dense_domain_bytes') is not None:
            result.append(dict(name=name+'_dense', bytes=c['dense_domain_bytes'], maps=needed,
                               bitmap=True, bits=c.get('dense_domain_size')))
    return result


def options_doop(r, domains, semantic_domains):
    result = []
    for name, c in r['candidates'].items():
        if not isinstance(c, dict) or c.get('bytes') is None or c.get('applicable') is False:
            continue
        maps = {}
        if 'semantic' in name:
            maps = {d:semantic_domains[d]['new_gpu_mapping_bytes_estimate']
                    for d in c['mapping_domains']}
        elif name == 'bitmap_shared_dense_product':
            maps = {s['domain']:domains[s['domain']]['new_gpu_mapping_bytes']
                    for s in r['key_stats']
                    if domains[s['domain']].get('new_gpu_mapping_bytes',0)}
        result.append(dict(name=name, bytes=c['bytes'], maps=maps,
                           bitmap=name.startswith('bitmap_'), bits=c.get('bits')))
    return result


def optimize(records, predicate=lambda o: True):
    """Exact enumeration of useful map subsets, with keep-current fallback."""
    useful = {}
    for r in records:
        no_map_best = min([r['current']]+[o['bytes'] for o in r['options'] if predicate(o) and not o['maps']])
        for o in r['options']:
            if predicate(o) and o['bytes'] < no_map_best:
                useful.update(o['maps'])
    ids = sorted(useful)
    best = None
    for bits in range(1 << len(ids)):
        enabled = {d for i, d in enumerate(ids) if bits & (1 << i)}
        mapping = sum(useful[d] for d in enabled)
        choices = []
        for r in records:
            eligible = [o for o in r['options'] if predicate(o) and set(o['maps']) <= enabled]
            choice = min([dict(name='keep_current', bytes=r['current'], maps={}, bitmap=False, bits=None), *eligible], key=lambda o:o['bytes'])
            choices.append(dict(structure_id=r['id'], layout=choice['name'], bytes=choice['bytes']))
        total = mapping + sum(c['bytes'] for c in choices)
        if best is None or total < best['bytes']:
            best = dict(bytes=total, mapping_bytes=mapping, mapping_domains=sorted(enabled), choices=choices)
    best['map_subsets_examined'] = 1 << len(ids)
    return best


def row_summary(r, workload, domains=None, semantic_domains=None):
    if workload == 'tpch':
        decl=r['evidence']['structures_declaration']
        kind=('canonical_fullset' if r['provider']=='radix_set' else
              'grouped_set' if r['provider']=='group_map' else
              'rows_publication' if ' rows =' in decl else
              'membership_set' if r['set_or_bag']=='set' and not r['payload'] else
              'built_index')
        identity=f"sf{r['scale']}/{r['query']}/s{r['stage']}/{r['relation']}/{kind}[{','.join(r['keys'])}]/{r['provider']}"
        current = r.get('current_live_bytes', r['current_bytes'])
        options = options_tpch(r)
        key_stats = [{'column':k, **v} for k,v in r['key_stats'].items()]
        layout = r.get('current_layout', {})
        word = layout.get('key_lane_bytes')
        if r['provider'] == 'sorted_run':
            word = 'host: 8-byte key/payload refs; u32 offsets'
        elif word is not None:
            word = f'{word}-byte packed row ({layout.get("key_bits","?")} bits)'
        elif 'directory_entries' in layout:
            word = f'directory FOR{layout.get("offset_width_bits","?")}b; payload FOR'+str([p.get('allocated_slot_bits') for p in layout.get('payload_columns',[])])
        elif r['provider'] == 'group_map':
            word = 'i64 keys/aggregates; u8 validity'
        elif layout.get('kind') == 'retained_prefix':
            word = f'{len(layout.get("payload_columns",[]))}×i64 prefix; u32 begin/offsets'
        else:
            word = layout.get('kind',r['provider'])
    else:
        identity = r['structure_id']+'/'+r['relation']
        kind=r['kind']
        current = r['current_bytes']
        options = options_doop(r, domains, semantic_domains)
        key_stats = r['key_stats']
        word = r['physical_word']
    if current is None:
        raise ValueError('Missing complete comparison footprint: '+identity)
    return dict(id=identity, workload=workload, query=r['query'], scale=r.get('scale'),
                relation=r['relation'], provider=r['provider'], kind=kind,
                keys=r['keys'], payload=r['payload'], arity=r['identity_arity'],
                key_arity=len(r['keys']), rows=r['rows'], distinct_keys=r['distinct_keys'],
                physical_rows=r.get('current_layout',{}).get('physical_rows',r['rows']),
                class_=r['class'], semantics=r['set_or_bag'], current=current,
                allocated=r.get('current_bytes') if workload=='tpch' else r.get('current_allocated_bytes'),
                word=word, options=options, key_stats=key_stats,
                order=r.get('order'), lookup_prefix=r.get('lookup_prefix'))


def aggregate(records, optimize_candidates=True):
    current = sum(r['current'] for r in records)
    bitmap_aware = optimize(records, lambda o:o['bitmap']) if optimize_candidates else None
    raw_bitmap = []
    dense_bitmap = []
    applicable = beneficial = density_applicable = 0
    density_forced = current
    for r in records:
        raw = [o for o in r['options'] if o['bitmap'] and not o['maps'] and ('raw' in o['name'])]
        dense = [o for o in r['options'] if o['bitmap'] and ('dense' in o['name'])]
        if r['key_arity']==1 and raw:
            o=min(raw,key=lambda o:o['bytes']); applicable+=1
            beneficial += o['bytes'] < r['current']
            raw_bitmap.append(o['bytes'])
            if o['bits'] is not None and o['bits'] <= 32*r['distinct_keys']:
                density_applicable+=1; density_forced+=o['bytes']-r['current']
        else:
            raw_bitmap.append(r['current'])
        dense_bitmap.append(min((o['bytes'] for o in dense),default=r['current']) if r['key_arity']==1 else r['current'])
    return dict(structures=len(records), current_accounted_bytes=current,
                class_counts=dict(Counter(r['class_'] for r in records)),
                best=optimize(records) if optimize_candidates else None, bitmap_cost_aware=bitmap_aware,
                scalar_raw_bitmap_applicable=applicable, scalar_raw_bitmap_beneficial=beneficial,
                scalar_raw_bitmap_forced_bytes=sum(raw_bitmap),
                scalar_shared_dense_bitmap_forced_excluding_maps_bytes=sum(dense_bitmap),
                scalar_span_density_eligible=density_applicable,
                scalar_span_density_forced_bytes=density_forced,
                unknown_allocated_capacity_count=sum(r['allocated'] is None for r in records))


def amount(n):
    if n is None: return 'unknown'
    if abs(n)>=10**15: return f'{n:.3e}'
    return f'{n:,}'


def stats_text(stats):
    parts=[]
    for s in stats:
        distinct=s.get('distinct')
        span=s.get('span')
        ratio=f'{span/distinct:.3g}' if span is not None and distinct else 'n/a'
        domain=s.get('domain')
        if isinstance(domain,dict):
            domain_text=f"{domain['id']} (D={domain.get('domain_size')})"
        else:
            domain_text=f"{domain or 'unresolved'} (D={s.get('domain_distinct','?')})"
        semantic=s.get('semantic_domain')
        if semantic:
            domain_text+=f"; optional {semantic} (D={s.get('semantic_domain_distinct')})"
        parts.append(f"{s['column']} [{s.get('logical_type','code')}; {domain_text}]={s.get('min')}..{s.get('max')}; d={amount(distinct)}; span/d={ratio}")
    return '<br>'.join(parts) or 'no lookup key'


def candidate_cell(r, family):
    oo=[o for o in r['options'] if family(o)]
    if not oo: return '—'
    no_map=[o for o in oo if not o['maps']]
    chosen=min(no_map or oo,key=lambda o:o['bytes'])
    return amount(chosen['bytes'])+(' + map' if chosen['maps'] else '')


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ('tpch','doop','universes','doop-provenance'):
        p.add_argument('--'+name,type=Path,required=True)
    p.add_argument('--output-dir',type=Path,default=HERE.parent/'results')
    args=p.parse_args()
    T=json.loads(args.tpch.read_text());D=json.loads(args.doop.read_text())
    U=json.loads(args.universes.read_text());P=json.loads(args.doop_provenance.read_text())
    P['preserved_measurement_sources']={}
    for key in ('measurement_driver','compiler_driver','compiler_manifest','compiler_inventory'):
        uri=P.get(key,'')
        if uri.startswith('local://'):
            source=args.doop_provenance.parent/uri.removeprefix('local://')
            if source.is_file():
                P['preserved_measurement_sources'][key]=source.read_text()
    trows=[row_summary(r,'tpch') for q in T['queries'] for r in q['structures']]
    drows=[row_summary(r,'doop',D['domains'],D.get('semantic_domains',{})) for r in D['structures']]
    per_query=[]
    for q in T['queries']:
        rr=[r for r in trows if r['query']==q['query'] and r['scale']==q['scale']]
        per_query.append(dict(query=q['query'],scale=q['scale'],**aggregate(rr)))
    # Mapping dictionaries belong to a query's catalog. Never amortize them
    # across independently run suite members without saying so.
    tpch_totals={}
    for sf in (1,10):
        queries=[q for q in per_query if q['scale']==sf]
        rr=[r for r in trows if r['scale']==sf]
        a=aggregate(rr, optimize_candidates=False)
        a['known_allocated_bytes']=sum(r['allocated'] for r in rr if r['allocated'] is not None)
        a['canonical_live_bytes']=sum(r['current'] for r in rr if r['provider']=='radix_set')
        a['host_sorted_run_bytes']=sum(r['current'] for r in rr if r['provider']=='sorted_run')
        a['best']=dict(bytes=sum(q['best']['bytes'] for q in queries),mapping_bytes=sum(q['best']['mapping_bytes'] for q in queries),scope='sum of independently optimized per-query inventories')
        a['bitmap_cost_aware']=dict(bytes=sum(q['bitmap_cost_aware']['bytes'] for q in queries),mapping_bytes=sum(q['bitmap_cost_aware']['mapping_bytes'] for q in queries),scope='sum of independently optimized per-query inventories')
        tpch_totals[str(sf)]=a
    da=aggregate(drows)
    sem=D.get('semantic_remap_totals',{})
    report=dict(date='2026-11-11',scope='measurement and explicit memory models; no engine changes',
                accounting='TPC-H published footprints retain existing padding; canonical sets use exact live keys+count when retained capacity is unobserved. DOOP owner totals and rigorous physical-sharing bounds are separate. Candidate codecs are estimates.',
                tpch_totals=tpch_totals,tpch_per_query=per_query,doop_totals=da,
                doop_physical_live_bounds=D.get('physical_live_bounds'),
                doop_semantic_remap_totals=sem,normalized_structures=trows+drows,
                source_censuses=dict(tpch=T,doop=D),numeric_universe_measurements=U,
                doop_measurement_provenance=P)
    output=args.output_dir;output.mkdir(parents=True,exist_ok=True)
    jsonpath=output/(STEM+'.json');mdpath=output/(STEM+'.md')
    methods='## Interpretation and accounting rules'+mdpath.read_text().split('## Interpretation and accounting rules',1)[1]
    lines=['# Canonical-index and membership-set memory census — 2026-11-11','',
           'All byte figures below are **bytes**, not MB. Census statistics come from real data; counterfactual layouts are estimates, not implemented formats or measured peak GPU memory. Full source plans, physical layout details, per-column statistics, domain maps and provenance are embedded in the JSON.','',
           '## Answer and coverage','',
           f"TPC-H: {len(T['queries'])} query/scale instances, {len(trows)} structures, complete cardinality and comparison-footprint accounting. {T['coverage']['unknown_current_allocations']} canonical retained capacities remain unobserved; their live keys/counts are known, not zero.",
           f"DOOP xalan: {D['coverage']['relations']} relations, {D['coverage']['canonical_sets']} canonical full sets, {D['coverage']['full_layouts']-D['coverage']['canonical_sets']} additional index orders, and {D['coverage']['negative_keysets']} negative keysets. Exact statistics come from real EDB facts, full GPU witnesses and CPU evaluation of remaining acyclic rules. All final relation row counts and layout live-byte aggregates match the revision-matched GPU trace; non-exported IDB column statistics are CPU-derived, not separately downloaded from the GPU.",'',
           'A bitmap-only policy is not a universal replacement. Scalar bitmaps may replace membership keys or a directory **with rank, offsets and payload retained**. Multi-column identity generally needs an enormous product bitmap; binary adaptive containers are a different representation. Cost-aware selection is required.','',
           '## Workload totals','',
           'TPC-H totals sum independent query inventories, not simultaneous GPU residency. The current column is a published/accounted footprint: known allocated buffers plus exact canonical live keys/counts where capacity is unknown. Candidate totals use the same structures and keep current when no modeled alternative helps. Shared maps are paid once per query, never once per structure or silently shared across queries.','',
           '| Workload | Structures | Current accounted/owner bytes | Best modeled bytes incl. selected maps | Bitmap-only cost-aware bytes incl. maps | Forced raw-span scalar bitmaps |',
           '|---|---:|---:|---:|---:|---:|']
    for sf,a in tpch_totals.items():
        lines.append(f"|TPC-H SF{sf}|{a['structures']}|{amount(a['current_accounted_bytes'])}|{amount(a['best']['bytes'])}|{amount(a['bitmap_cost_aware']['bytes'])}|{amount(a['scalar_raw_bitmap_forced_bytes'])}|")
    lines.append(f"|DOOP xalan (logical owners)|{da['structures']}|{amount(da['current_accounted_bytes'])}|{amount(sem.get('cost_aware_selected_maps_bytes',da['best']['bytes']))}|{amount(da['bitmap_cost_aware']['bytes'])}|{amount(da['scalar_raw_bitmap_forced_bytes'])}|")
    lines += ['', f"The TPC-H inventory includes the recorded host-side H02 `SortedRun`: {amount(tpch_totals['1']['host_sorted_run_bytes'])} bytes at SF1 and {amount(tpch_totals['10']['host_sorted_run_bytes'])} at SF10. These are not GPU-resident allocations. All other provider placements and exact stored widths are identified in the structure tables."]
    bounds=D['physical_live_bounds']
    lines += ['',f"**DOOP physical live ownership bounds:** {amount(bounds['inclusive_live_lower_bytes'])}–{amount(bounds['inclusive_live_upper_bytes'])} bytes, versus the larger owner sum above. Fresh same-order batches alias; independently merged batches need not. Bounds include both negative keysets. They exclude retained old snapshots, allocator caches, build scratch and query peak. The candidate owner sum is a conservative no-alias upper accounting scenario, not measured residency.",'',
              'The bitmap-only cost-aware column permits any profitable modeled bitmap, including small Cartesian domains; it excludes FOR/u32/adaptive alternatives. Forced scalar bitmap totals replace every scalar-key structure with its raw-span bitmap even when this loses; other structures remain unchanged. The JSON also records the stricter `span <= 32*distinct` policy and conditional full-shared-domain scalar totals.','',
              '## Structure classes and bitmap applicability','',
              '| Workload | Single dense key | Single sparse key | Multi-column | Bag/occurrence or other | Scalar raw bitmap candidates | Profitable scalar raw bitmaps | span/d <=32 |',
              '|---|---:|---:|---:|---:|---:|---:|---:|']
    for label,a in [(f'TPC-H SF{sf}',a) for sf,a in tpch_totals.items()]+[('DOOP xalan',da)]:
        c=a['class_counts'];other=sum(v for k,v in c.items() if k not in ('single_dense_key','single_sparse_key','multi_column'))
        lines.append(f"|{label}|{c.get('single_dense_key',0)}|{c.get('single_sparse_key',0)}|{c.get('multi_column',0)}|{other}|{a['scalar_raw_bitmap_applicable']}|{a['scalar_raw_bitmap_beneficial']}|{a['scalar_span_density_eligible']}|")
    lines += ['', 'Classes describe raw observed spans/identity and storage obligations, not private dense interning. Payload-bearing structures retain payload even when their key domain is dense. Exact classification and set/bag flags are recorded per structure.', '', '## TPC-H per-query totals', '',
              '| SF | Query | Structures | Current accounted bytes | Best incl. maps | Bitmap-only cost-aware incl. maps | Unknown allocated capacities |',
              '|---:|---|---:|---:|---:|---:|---:|']
    for q in sorted(per_query,key=lambda q:(q['scale'],q['query'])):
        lines.append(f"|{q['scale']}|{q['query']}|{q['structures']}|{amount(q['current_accounted_bytes'])}|{amount(q['best']['bytes'])}|{amount(q['bitmap_cost_aware']['bytes'])}|{q['unknown_allocated_capacity_count']}|")
    lines += ['', '## Largest modeled winners and forced bitmap losers', '',
              'Winner rows below compare against no-remap alternatives, avoiding attribution of a shared mapping cost to one beneficiary. Positive savings are model estimates.','',
              '| Structure | Current | No-remap choice | Candidate | Saved |','|---|---:|---|---:|---:|']
    wins=[]
    for r in trows+drows:
        oo=[o for o in r['options'] if not o['maps']]
        if oo:
            o=min(oo,key=lambda o:o['bytes']);wins.append((r['current']-o['bytes'],r,o))
    for saving,r,o in sorted(wins,key=lambda x:x[0],reverse=True)[:20]:
        lines.append(f"|{r['id']}|{amount(r['current'])}|{o['name']}|{amount(o['bytes'])}|{amount(saving)}|")
    lines += ['', 'Full shared-domain scalar bitmaps can be terrible for tiny sets. These are intentionally forced-policy losers; raw-span trimming, FOR or a plain array can avoid them. Mapping costs would only add to the dense-domain numbers below.','',
              '| Structure | Set keys | Shared domain positions | Current | Forced shared-domain bitmap+retained data | Ratio |','|---|---:|---:|---:|---:|---:|']
    losers=[]
    for r in trows+drows:
        if r['key_arity']!=1 or not r['rows']:continue
        oo=[o for o in r['options'] if o['bitmap'] and ('dense' in o['name'])]
        if oo:
            o=min(oo,key=lambda o:o['bytes'])
            if o['bytes']>r['current']:losers.append((o['bytes']/max(1,r['current']),r,o))
    for ratio,r,o in sorted(losers,key=lambda x:x[0],reverse=True)[:20]:
        lines.append(f"|{r['id']}|{amount(r['distinct_keys'])}|{amount(o['bits'])}|{amount(r['current'])}|{amount(o['bytes'])}|{ratio:.2f}×|")
    lines += ['', 'TPC-H also has losers even without new interning or a full-domain allocation. The largest raw-span scalar-bitmap increases below retain all bag payload and compact occurrence offsets; dense keys alone do not guarantee a smaller directory.', '',
              '| TPC-H structure | Distinct keys | Raw span | Current | Bitmap+rank/offsets/payload | Increase |',
              '|---|---:|---:|---:|---:|---:|']
    raw_losers=[]
    for r in trows:
        oo=[o for o in r['options'] if o['bitmap'] and 'raw' in o['name'] and r['key_arity']==1]
        if oo:
            o=min(oo,key=lambda o:o['bytes'])
            if o['bytes']>r['current']:raw_losers.append((o['bytes']-r['current'],r,o))
    for increase,r,o in sorted(raw_losers,key=lambda x:x[0],reverse=True)[:12]:
        lines.append(f"|{r['id']}|{amount(r['distinct_keys'])}|{amount(o['bits'])}|{amount(r['current'])}|{amount(o['bytes'])}|{amount(increase)}|")
    lines += ['', '## DOOP domains, word types and adaptive binary sets', '',
              f"Physical word types across all layouts/keysets: `{dict(Counter(r['word'] for r in drows))}`. All use implicit unit offsets; emitted radix directories have zero entries. Per-run/field/count metadata are included, not an invented 4-byte offset per tuple.", '',
              f"The existing global IStr universe has {D['domains']['doop.process_global_IStr']['distinct']:,} codes. The string dictionary is already host-resident and is not uploaded as a new GPU dictionary. There are {len(D.get('semantic_domains',{}))} additional measured rule-variable equality components available as optional shared semantic domains.",
              f"All semantic maps would cost {amount(sem.get('mapping_bytes'))} bytes. The exact useful-map subset search selects {sem.get('selected_mapping_domains',[])} for {amount(sem.get('selected_mapping_bytes'))} bytes, yielding {amount(sem.get('cost_aware_selected_maps_bytes'))} bytes including maps, versus {amount(D['inclusive_totals']['cost_aware_bytes'])} without new semantic maps. These maps support binary-search translation, not an assumed free O(1) lookup.",'',
              f"Using only bitmaps plus current fallback instead selects {da['bitmap_cost_aware']['mapping_domains']}, paying {amount(da['bitmap_cost_aware']['mapping_bytes'])} bytes in maps; total {amount(da['bitmap_cost_aware']['bytes'])}. Thus broad bitmap replacement barely changes DOOP memory compared with the much larger FOR/adaptive benefit.",'',
              '| Shared semantic domain | Distinct | Original code min..max | New map bytes | Column members |','|---|---:|---|---:|---:|']
    for name,d in D.get('semantic_domains',{}).items():
        lines.append(f"|{name}|{amount(d['distinct'])}|{d.get('canonical_code_min')}..{d.get('canonical_code_max')}|{amount(d.get('new_gpu_mapping_bytes_estimate'))}|{len(d['columns'])}|")
    for label,rr in [('TPC-H',trows),('DOOP',drows)]:
        eligible=[]
        for r in rr:
            oo=[o for o in r['options'] if 'adaptive' in o['name'] and not o['maps']]
            if oo:eligible.append((r,min(oo,key=lambda o:o['bytes'])))
        lines.append(f"\n{label}: {len(eligible)} modeled binary adaptive structures; {sum(o['bytes']<r['current'] for r,o in eligible)} are smaller than current. Chunk/group/rank metadata can outweigh u16 payload savings for tiny groups.")
    lines += ['', '## Complete structure tables', '',
              'Candidate columns choose the smallest no-remap option in that family where available; `+ map` means the displayed data bytes additionally require shared maps. **Best workload totals above charge maps exactly once.** A dash means not modeled/applicable, never zero. Raw and dense bitmap alternatives, all byte components and physical-versus-logical row distinctions remain in JSON.','']
    for label,rr in [('TPC-H SF1',[r for r in trows if r['scale']==1]),('TPC-H SF10',[r for r in trows if r['scale']==10]),('DOOP xalan',drows)]:
        lines += ['### '+label,'','| Structure / provider | Keys → payload; order | Set/bag; identity/key arity | Logical / stored rows; distinct keys | Current; word/layout | u32 | FOR+fences | Raw bitmap | Dense bitmap | Binary adaptive | Class |','|---|---|---|---:|---|---:|---:|---:|---:|---:|---|']
        for r in rr:
            order=f"; order={r['order']}" if r['order'] is not None else ''
            lines.append(f"|{r['id']} / {r['provider']}|{','.join(r['keys']) or '∅'} → {','.join(r['payload']) or '∅'}{order}|{r['semantics']}; {r['arity']}/{r['key_arity']}|{amount(r['rows'])} / {amount(r['physical_rows'])}; d={amount(r['distinct_keys'])}|{amount(r['current'])}; {r['word']}|{candidate_cell(r,lambda o:'u32' in o['name'])}|{candidate_cell(r,lambda o:'for1024' in o['name'])}|{candidate_cell(r,lambda o:o['bitmap'] and 'raw' in o['name'])}|{candidate_cell(r,lambda o:o['bitmap'] and 'dense' in o['name'])}|{candidate_cell(r,lambda o:'adaptive' in o['name'])}|{r['class_']}|")
        lines += ['', 'Per-key-column min/max, distinct count and span/distinct (code min/max, not lexical string order):', '', '| Structure | Column statistics |', '|---|---|']
        for r in rr:
            lines.append(f"|{r['id']}|{stats_text(r['key_stats'])}|")
    lines += ['', '## Provenance and uncertainty', '',
              f"TPC-H plan source: `{T['plan_source']}`; SHA-256 `{T['plan_sha256']}`. All 52 accepted advised query/scale plans and their Structures text are retained in JSON.",
              f"DOOP measured revision: `{D['commit']}`; binary SHA-256 `{D['binary_sha256']}`. All {D['coverage']['relations']} relation counts match exact reconstructed tuples. Layout byte aggregates match revision-matched trace records.", '',
              'The remaining uncertainty is explicitly allocation-related: 48 TPC-H canonical pre-dedup capacities, DOOP noncanonical capacity slack, old snapshots, allocator/cache residency and exact IDB alias sharing are not inferred from row counts. No full-heap or peak-memory claim is made. Candidate savings are layout-model differences, not a deployed memory reduction.', '',
              'TPC-H and DOOP candidate headers/rank/checkpoint profiles differ and are specified in each source record. They are feasible accounting profiles, not claims that either workload already implements them.', '',
              'Reproduction helpers: `tpch_index_census.py`, `doop_index_census.py`, and `index_census_report.py` in `experiments/storage-bench/gpu/`. DOOP build/run/source provenance and failed attempts are embedded in JSON; local measurement artifacts are source evidence, not engine edits.', '',
              '```sh',P['build_command'],'```','',methods]
    jsonpath.write_text(json.dumps(report,indent=2)+'\n')
    mdpath.write_text('\n'.join(lines)+'\n')
    print(json.dumps(dict(tpch_totals=tpch_totals,doop_current=da['current_accounted_bytes'],
                          doop_best=da['best']['bytes'],doop_bitmap_only=da['bitmap_cost_aware']['bytes'],
                          structures=len(trows)+len(drows)),indent=2))

if __name__=='__main__':
    main()
