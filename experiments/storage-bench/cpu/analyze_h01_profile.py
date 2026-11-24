#!/usr/bin/env python3
"""Conservative phase and exclusive self-symbol attribution of recorded samples."""
import collections
import json
import pathlib
import re
import subprocess
root=pathlib.Path('/data/mini_linq/experiments/storage-bench/results')
prefix=root/'cpu-h01-perf-2026-10-29'
header=re.compile(r'^\s*\S+\s+\d+/\d+\s+(\d+\.\d+):\s+(\d+)\s+(\S+):\s*$')
frame=re.compile(r'^\s+[0-9a-f]+\s+(.+?)\s+\((.*)\)$')
samples=[];current=None;symbols=set()
with pathlib.Path(str(prefix)+'.script.txt').open() as f:
    for line in f:
        h=header.match(line)
        if h:
            if current:samples.append(current)
            current={'timestamp':float(h[1]),'weight':int(h[2]),'event':h[3],'frames':[]}
            continue
        m=frame.match(line)
        if m and current:
            sym=m[1];current['frames'].append(sym);symbols.add(sym)
    if current:samples.append(current)
ordered=sorted(symbols)
demangled=subprocess.run(['c++filt','-s','rust'],input='\n'.join(ordered)+'\n',text=True,stdout=subprocess.PIPE,check=True).stdout.splitlines()
assert len(demangled)==len(ordered)
# GNU rust demangling retains crate disambiguators, unlike rustc-demangle.
# Remove only hexadecimal crate hashes for stable role/phase matching.
names={raw:re.sub(r'\[[0-9a-f]{8,}\]','',pretty) for raw,pretty in zip(ordered,demangled)}
def phase(fs):
    if any('tpch_derived_retail_h01::query::' in f and ('::run' in f or '::default' in f or '::no_reduce' in f) for f in fs):return 'query'
    if any('tpch_derived_retail_h01::query::load' in f or 'cq_load::load' in f or f.startswith('parquet::') or 'parquet::file::' in f for f in fs):return 'load'
    return 'other/unclassified'
def role(sym,fs):
    if any('handle_mm_fault' in f or 'do_user_addr_fault' in f or 'exc_page_fault' in f for f in fs):return 'page-fault-context sample'
    if any(x in sym.lower() for x in ('malloc','free','calloc','realloc','alloc::alloc','alloc_zeroed','munmap','mmap')):return 'allocation/free'
    if any(x in sym.lower() for x in ('memcpy','memmove','memset')):return 'memory copy/zero'
    if 'memcmp' in sym or 'bcmp' in sym:return 'string/key comparisons'
    if 'hash_one' in sym and 'cq_storage::hash_state::HashState' in sym:return 'dedup hashing'
    if sym.startswith('ahash::') and any('HashState' in f for f in fs):return 'dedup hashing'
    if 'core::hash::sip' in sym or ('hash_one' in sym and 'std::hash::random::RandomState' in sym):return 'group hashing (SipHash)'
    if 'hashbrown' in sym:
        if 'HashState' in sym or 'reserve_rehash' in sym or 'cq_load::scalar::Decimal' in sym:return 'dedup table/probe/growth'
        if 'rustc_entry' in sym or 'RandomState' in sym:return 'group table/probe'
        return 'hash table (unresolved role)'
    if 'tpch_derived_retail_h01::query::' in sym or 'try_fold' in sym or 'Iterator' in sym:return 'fused scan/decimal/group reduction'
    if 'cq_load::scalar::' in sym:return 'scan/decimal'
    return 'other/unclassified'
phase_counts=collections.defaultdict(lambda:[0,0]);fault_counts=collections.defaultdict(lambda:[0,0]);role_counts=collections.defaultdict(lambda:[0,0]);symbol_counts=collections.defaultdict(lambda:[0,0]);rehash_cycles=0
for sample in samples:
    fs=[names[x] for x in sample['frames']];ph=phase(fs);w=sample['weight'];sym=fs[0] if fs else '[no frame]'
    if sample['event'].startswith('cycles'):
        phase_counts[ph][0]+=w;phase_counts[ph][1]+=1
        if ph=='query':
            r=role(sym,fs);role_counts[r][0]+=w;role_counts[r][1]+=1;symbol_counts[(sym,r)][0]+=w;symbol_counts[(sym,r)][1]+=1
            if r=='dedup hashing' and any('reserve_rehash' in x for x in fs):rehash_cycles+=w
    elif sample['event']=='page-faults':fault_counts[ph][0]+=w;fault_counts[ph][1]+=1

def table(counts):
    total=sum(x[0] for x in counts.values())
    return {k:{'weight':v[0],'samples':v[1],'percent':100*v[0]/total if total else 0} for k,v in sorted(counts.items(),key=lambda kv:-kv[1][0])}
for zero_role in ('allocation/free','memory copy/zero','scan/decimal'):
    role_counts.setdefault(zero_role,[0,0])
query_total=sum(v[0] for v in role_counts.values())
top=[{'symbol':sym,'role':r,'weight':v[0],'samples':v[1],'percent':100*v[0]/query_total} for (sym,r),v in sorted(symbol_counts.items(),key=lambda kv:-kv[1][0])[:25]]
analysis={'provenance':json.loads(pathlib.Path(str(prefix)+'.provenance.json').read_text()),'worker_metrics':json.loads(pathlib.Path(str(prefix)+'.metrics.json').read_text()),'cycle_phases':table(phase_counts),'query_roles':table(role_counts),'top_query_symbols':top,'fault_phases':table(fault_counts),'rehash_dedup_hash_query_cycle_percent':100*rehash_cycles/query_total if query_total else 0,'sample_count':len(samples),'demangler':'c++filt -s rust','phase_rules':'query run/default/no_reduce callchain first, then query::load/cq_load::load/parquet callchain, otherwise unclassified; exclusive self-symbol roles','limitations':'sample-period-weighted estimates, not exact operation counts; mixed fused functions cannot isolate scan vs reduction; user-cycle event cannot measure all kernel handling time; page-fault events reported separately'}
pathlib.Path(str(prefix)+'.analysis.json').write_text(json.dumps(analysis,indent=2)+'\n')
print(json.dumps({'phase_shares':analysis['cycle_phases'],'query_roles':analysis['query_roles'],'rehash_hash_percent':analysis['rehash_dedup_hash_query_cycle_percent'],'worker_metrics':analysis['worker_metrics']},indent=2))
