#!/usr/bin/env python3
"""Append M6b to the existing M6 report without assuming source-key uniqueness."""
import json
import pathlib
import subprocess
ROOT=pathlib.Path('/data/mini_linq/experiments/storage-bench/results')

def render(doc):
    m=doc['metadata'];rows=doc['rows'];order=m['sorted_nondecreasing_verified']
    lines=['## M6b: order-verified run distinct versus hash and packed-sort distinct','',
    '**No uniqueness assumption.** S1 checks only nondecreasing lexicographic `(l_orderkey,l_linenumber)` in one sequential resident-data pass. Equal prefixes are permitted. '
    'Within every equal-prefix run, S2/S3 compare a candidate full source tuple against all earlier tuples in that run. Thus X,Y,X suppresses the second X; merely comparing adjacent full tuples would be incorrect. '
    'Worst-case work inside a length-L run is quadratic; this is a measured choice for this input, not a general asymptotic improvement over hashing.','',
    f"**S1 result:** nondecreasing verification {'passed' if order else 'failed'}. Uniqueness is neither tested nor assumed. "
    'If verification fails, S2 falls back to global String/hash distinct, S3 to global reserved code/hash distinct; H/R also use the global serial fallback instead of unsafe independent chunk distinct. '
    'Tests covered nonadjacent duplicates within a prefix run, a run crossing 65,536-row task boundaries, excluded dates, and unsorted fallback.','',
    't24 uses fixed 65,536-row tasks extended to complete equal-prefix runs. H/R use the same partitioning as S2/S3, so duplicates cannot straddle independent sets. '
    '**t24 H is this common parallelized kernel, not a claim about the generated engine’s parallel configuration.** t1 uses the full input in one task. '
    'S1 and packing-bound construction remain single-thread sequential load-time passes even when shown in the t24 experiment group; their facts are reused by query samples.','',
    'P/P′ require no input ordering: filter, encode all seven source columns into an injective u128, sort, adjacent-unique, then group directly from packed fields. '
    'All seven fields participate in equality, not just the prefix. Load-time min/max bounds describe representability; this input fits u128. '
    'P is stable LSD radix with 8-bit digits; t24 uses fixed-chunk histograms, exclusive per-bucket/chunk offsets and parallel scatter. '
    'P′ uses `sort_unstable` / Rayon `par_sort_unstable` (Rayon controls its comparison-sort scheduling). '
    'Adjacent-unique emits a selection mask rather than copying a compacted key array; grouping consumes only marked keys.','',
    '| Field | Base | Width bits | Shift |','|---|---:|---:|---:|']
    for name,base,width,shift in zip(m['for_fields'],m['for_bases'],m['for_widths'],m['for_shifts']):
        lines.append(f'| {name} | {base} | {width} | {shift} |')
    lines += ['',f"Total: **{m['packed_bits']} bits** in a 128-bit word; {m['radix_passes']} radix passes. Bounds are computed over all resident input rows, before predicate filtering.",'',
    '**Protocol:** five warmups, 21 samples, rotated interleaving, paired-bootstrap ratios to the H measured in the same thread-count group. Exact per-sample answers matched the independent DuckDB SF1 oracle. '
    'No changes were selected by observed performance. Input construction and load-fact calculation are excluded from query times and reported separately. '
    'Query allocations, scratch/mask creation and destruction are included; phase residuals include dispatch/timer and final vector destruction. '
    'The existing environment checks record clocks/load/top processes/GPU tenants after each round and repeat contaminated groups.','',
    'Ratios to DuckDB use the **supplied historical t1 819 ms** as a fixed denominator, not a newly measured control or a paired comparison. '
    'Their CIs reflect only this experiment’s uncertainty; t24/DuckDB-t1 ratios are deliberately cross-thread reference ratios.','',
    '### M6b query measurements','',
    '| t | Variant | Median ms | 95% CI ms | Ratio to same-t H (95% CI) | Ratio to DuckDB t1 (95% CI) |','|---:|---|---:|---|---|---|']
    for r in rows:
        if r['sample_scope']!='resident query':continue
        lo,hi=r['ci95_ns'];rl,rh=r['ratio_to_H_ci95'];dl,dh=r['ratio_to_duckdb_t1_ci95']
        lines.append(f"| {r['threads']} | {r['variant']} | {r['median_ns']/1e6:.3f} | [{lo/1e6:.3f}, {hi/1e6:.3f}] | {r['ratio_to_H']:.4f} [{rl:.4f}, {rh:.4f}] | {r['ratio_to_duckdb_t1']:.4f} [{dl:.4f}, {dh:.4f}] |")
    lines += ['', '### Separate load-time representation checks', '', '| Execution configuration | Pass | Actual threads | Median ms | 95% CI ms |','|---:|---|---:|---:|---|']
    for r in rows:
        if r['sample_scope']=='load-time pass':
            lo,hi=r['ci95_ns'];lines.append(f"| {r['threads']} | {r['variant']} | {r['execution_threads']} | {r['median_ns']/1e6:.3f} | [{lo/1e6:.3f}, {hi/1e6:.3f}] |")
    lines += ['', 'These are resident-data construction costs, not file I/O timings. They are paid once per immutable loaded representation, not once per query. S2/S3 depend on S1; P/P′ depend on the FOR-bound pass. H/R t24 also depend on S1 for the common prefix-aligned partitioning.', '',
    '### Packed sort / unique / group phase times', '', '| t | Variant | Phase | Median ms | 95% CI ms |','|---:|---|---|---:|---|']
    for r in rows:
        for phase in r.get('phases',[]):
            lo,hi=phase['ci95_ns'];lines.append(f"| {r['threads']} | {r['variant']} | {phase['phase']} | {phase['median_ns']/1e6:.3f} | [{lo/1e6:.3f}, {hi/1e6:.3f}] |")
        if 'phases' in r:
            lines.append(f"| {r['threads']} | {r['variant']} | outer overhead and final drop | {r['overhead_and_drop_median_ns']/1e6:.3f} | — |")
    lines += ['', 'Phase medians need not sum to the median total; every raw phase sample is retained in JSON.', '', '### M6b interpretation', '']
    for t in (1,24):
        rs={r['variant']:r for r in rows if r['threads']==t and r['sample_scope']=='resident query'}
        best=min(rs.values(),key=lambda r:r['median_ns'])
        lines.append(f"- **t{t} fastest median:** {best['variant']} at {best['median_ns']/1e6:.3f} ms ({best['ratio_to_H']:.4f}× same-t H). This is a median ranking; the individual CIs above determine whether differences are resolved.")
        radix=rs['P_packed_radix'];cmp=rs['Pprime_packed_comparison'];reserved=rs['R_reserved_hash_codes']
        lines.append(f"- t{t}: radix total/comparison-sort total = {radix['median_ns']/cmp['median_ns']:.3f}; radix/reserved-code-hash total = {radix['median_ns']/reserved['median_ns']:.3f}. These descriptive median ratios do not carry a separate significance claim.")
        radix_sort=next(p['median_ns'] for p in radix['phases'] if p['phase']=='sort')
        compare_sort=next(p['median_ns'] for p in cmp['phases'] if p['phase']=='sort')
        if radix_sort>compare_sort:
            lines.append(f"- t{t}: comparison sort also wins the isolated sort phase ({compare_sort/1e6:.3f} versus radix {radix_sort/1e6:.3f} ms), contrary to assuming radix must win integer-key sorting. [INFERENCE] Ten radix passes, scratch initialization/scatter traffic, and the library sort's implementation/input sensitivity can explain this result; no counter-level attribution was collected for these variants.")
        verify=next(r for r in rows if r['threads']==t and r['variant']=='S1_load_order_verification')
        s3=rs['S3_ordered_codes_array']
        lines.append(f"- t{t}: S1 median + S3 median = {(verify['median_ns']+s3['median_ns'])/1e6:.3f} ms as a first-query cost illustration; not a separately timed sum or a CI estimate.")
    lines += ['- S2/S3 measure elimination of a global hash set under a verified ordering fact, **not** elimination of DISTINCT by presumed key uniqueness. Nonadjacent duplicate payloads within a prefix run remain correct.',
              '- P/P′ retain fully general order-independent distinct semantics within the verified 128-bit packing domain. Their sort/unique/group costs must be weighed against reserved hashing rather than against an illegal no-distinct shortcut.',
              '- At t24, R exploits verified prefix routing into independent sets while P/P′ perform a global order-independent sort. Thus the t24 ratios do not establish a winner between two unrestricted unordered-input parallel algorithms. P/P′ also include standard collection/allocation costs in encoding; these are measured implementations, not claims of optimal radix or packing performance.',
              '- [INFERENCE] Packing can reduce key footprint and random pointer accesses, while radix costs multiple full passes and scratch writes. Comparison sort can respond differently to this input’s value distribution. These mechanisms explain candidate tradeoffs, not measured hardware-counter attribution.',
              '- [INFERENCE] t24 changes scheduling, memory bandwidth and partition working-set size; scaling is not attributable solely to the number of CPU cores.', '', '### M6b environment and reproduction', '']
    accepted=[w for w in doc['windows'] if w.get('accepted')]
    lines.append(f"{len(accepted)} accepted thread-count groups; {len(doc['windows'])-len(accepted)} busy/rejected records. Small correctness cases and every retained sample were exact.")
    for w in accepted:lines.append(f"t{w['threads']}: {w['before']['utc'].strip()} through {w['after']['utc'].strip()}.")
    lines += ['', '```sh', "nice -n 19 env CARGO_TARGET_DIR=/data/mini_linq/experiments/storage-bench/cpu/target RUSTFLAGS='-C target-cpu=native' cargo build --release -j1 --manifest-path experiments/storage-bench/cpu/Cargo.toml --bin h01b", 'experiments/storage-bench/cpu/target/release/h01b --small', '# Only in a quiet authorized measurement window:', 'experiments/storage-bench/cpu/target/release/h01b --machine-quiet', 'python3 experiments/storage-bench/cpu/summarize_h01b.py', '```', '', 'Supplemental raw matrix: `cpu-h01-order-distinct-2026-10-29.json`; it is also embedded as `m6b` in the main M6 JSON.']
    return '\n'.join(lines)

if __name__=='__main__':
    main=ROOT/'cpu-h01-decomposition-2026-10-29.json'
    data=json.loads(main.read_text());extra=json.loads((ROOT/'cpu-h01-order-distinct-2026-10-29.json').read_text())
    assert extra['metadata']['oracle_exact'] and all(r['exact'] and len(r['samples_ns'])==21 for r in extra['rows'])
    data['m6b']=extra;main.write_text(json.dumps(data,indent=2)+'\n')
    subprocess.run(['python3',str(pathlib.Path(__file__).with_name('summarize_h01.py'))],check=True)
    print('Appended M6b:',len(extra['rows']),'measured query/load-pass rows')
