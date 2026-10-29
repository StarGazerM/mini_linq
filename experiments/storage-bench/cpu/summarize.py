#!/usr/bin/env python3
"""Render only observed benchmark data; no inferred numerical results."""
import collections
import json
import pathlib
import re
root = pathlib.Path('/data/mini_linq/experiments/storage-bench/results')
p = root / 'cpu-2026-10-28.json'
doc = json.loads(p.read_text())
rows = doc['rows']
groups = collections.defaultdict(list)
for r in rows:
    groups[(r['experiment'], r['case'], r['threads'])].append(r)
lines = ['# CPU storage codec measurements — 2026-10-28', '',
         '## Scope and protocol', '',
         'Rust standalone crate `experiments/storage-bench/cpu/`; AMD Threadripper PRO 5945WX (12 cores / 24 logical CPUs), resident data, t1 and Rayon t24. '
         'Release build: `RUSTFLAGS="-C target-cpu=native"`, `opt-level=3`, thin LTO, one codegen unit; compiled with `nice -n 19` and `-j1`. '
         'The JSON contains compiler/CPU/driver metadata, current CPU MHz, GPU clock reports, `uptime`, top CPU processes and compute tenants before/after each interleaved measurement group. '
         'Groups with any GPU compute tenant or an external process at ≥20% lifetime-average CPU according to `ps` are deferred/discarded and repeated. '
         'This boundary check cannot detect all transient or low-average contention.', '',
         'Every variant has five warmups and 21 retained samples, with a rotated variant order per round. '
         'Every sample and warmup is checked exactly against the plain reference outside timing. '
         'CIs use 2,000 deterministic nonparametric bootstrap resamples; ratio CIs resample paired round indices and divide medians. '
         'Ratios below 1 favor the encoded variant. CIs characterize these runs, not cross-machine reproducibility.', '',
         'Bit packing: 1024-row blocks, 16 interleaved u64 lanes; const-generic unpack specializations for each width 0–64. '
         'The inner contiguous lane loop has uniform shifts, making vectorization possible without per-value width dispatch. '
         'Byte offsets are u8/u16/u32 (u64 fallback only exercised by codec correctness tests), with per-block minimum base. '
         'Dictionary codes use one global ceil(log2 distinct) width; byte dictionary codes use global u8/u16/u32 width. '
         'All dictionaries are sorted. M1 measures reconstruction before predicates, not specialized code-domain predicates.', '',
         'Tuning effort: the same block size and fixed chunking were selected once, with no measured-result-driven tuning for any codec. '
         'Native plain scans directly access native arrays; compressed scans unpack stack blocks. '
         'M1/M5 task chunks are 65,536 rows, M2 chunks are 1,024 probes, M3 merge chunks are 65,536 output rows and decode/encode blocks are 1,024 rows. '
         'No thread affinity or NUMA binding was applied. M3 uses preallocated scratch and output buffers for both variants; re-encoding recomputes block min/max and checks the preconfigured output width.', '',
         'M1 Q6 uses real SF1 data and a tenfold tile, with shipdate [1994-01-01, 1995-01-01), discount [5,7] cents, and quantity <2400 cents. '
         'Extendedprice is accessed only for qualifying rows. Synthetic values are deterministic uniform w-bit integers in [0,2^w); '
         'actual achieved selectivity is included in each case (the w=4 domain cannot express 1% or 15% exactly). '
         'M2 uses real orderkey→row positions and sorted partkey→suppkey, identical binary-search directories, 65,536 deterministic probes with exactly half hits; misses are above the maximum key. '
         'All six payload×offset combinations are interleaved in each M2 group; its common ratio baseline is plain payload with u32 offsets. '
         'An earlier offset-separated run was superseded, not used for the reported comparisons, and retained in the JSON for provenance. '
         'M3 uses sorted u64 sequences 3*i and 29*i+1, and verifies every merged and decoded re-encoded output element. '
         'M5 uses the actual read-only DuckDB SF1 returnflag/linestatus columns, exported in dbgen order. '
         'The baseline hashes borrowed String-backed key pairs (no per-row string cloning), while u8 combined codes directly index six count/sum groups; sums use extendedprice.', '',
         '**Bandwidth qualification:** reported effective GB/s counts logical encoded input accesses, base/width metadata, and dictionary dereferences, not measured DRAM traffic. '
         'M1 predicate columns are fully read and prices are late materialized. Repeated scalar accesses count repeatedly. '
         'Dictionary lookup locality, cache-line traffic, allocator headers, scratch writes, and hardware prefetch traffic are not observed. '
         'M3 GB/s counts input bytes only, excluding output and decode intermediates; M2 reports probes/s and does not claim bandwidth. '
         'Resident byte counts are encoded input sizes, not complete process RSS. Compression-build costs and load/transfer are excluded.', '',
         '## Exactness and environment', '']
lines.append(f'{len(rows)} variant measurements retained; all exactness checks passed. Small-size checks also covered all codec widths 0–64, partial blocks, dictionary mappings, and merge co-rank partitions.')
tenants = []
for window in doc['windows']:
    for side in ('before', 'after'):
        text = window[side]['gpu_processes'].strip()
        if text:
            tenants.append((window['experiment'], window['case'], window['threads'], side, text))
if tenants:
    lines += ['', '**GPU tenant observations:** these CPU measurements encountered GPU compute processes; GPU device timings were not performed here. See JSON windows for the full process records.']
    lines += ['```', *[str(t) for t in tenants], '```']
else:
    lines += ['', 'No GPU compute applications were reported at the recorded boundaries.']
loads = [float(w[s]['loadavg'].split()[0]) for w in doc['windows'] for s in ('before', 'after')]
if loads:
    lines.append(f'Recorded one-minute machine-load range: {min(loads):.2f}–{max(loads):.2f}; it includes benchmark worker activity and is not an independent contention measure.')
rejected = sum(not w.get('accepted', True) for w in doc['windows'])
lines.append(f'{rejected} busy/contaminated group-boundary records were rejected; only accepted measurements contribute to medians and CIs.')
times = [w[s]['utc'].strip() for w in doc['windows'] for s in ('before', 'after')]
lines.append(f'Observed UTC interval: {min(times)} through {max(times)}. The report filename uses the local session date (2026-10-28).')
lines += ['', '## Per-experiment winners', '', '| Experiment / case | Threads | Fastest median | Median ms | Ratio to plain (95% CI) |', '|---|---:|---|---:|---|']
for (exp, case, t), rs in groups.items():
    if exp == 'M1-count':
        continue
    best = min(rs, key=lambda r:r['median_ns'])
    lo, hi = best['ratio_ci95']
    lines.append(f"| {exp}: {case} | {t} | {best['variant']} | {best['median_ns']/1e6:.4f} | {best['ratio_to_plain']:.3f} [{lo:.3f}, {hi:.3f}] |")
lines += ['', '## Decode cost versus bandwidth: observed width sweep', '',
          'A width is a clear encoded win only if its ratio CI is entirely below 1; a clear loss has CI entirely above 1. '
          'No interpolated crossover is claimed between the seven tested widths. The outcomes can be nonmonotone because cache residency, storage byte-width boundaries, selectivity, and thread overhead change independently.', '',
          '| Threads | Rows | Selectivity target | Codec | Clear wins (bits) | Overlapping CI (bits) | Clear losses (bits) |', '|---:|---:|---:|---|---|---|---|']
sweeps = collections.defaultdict(dict)
for r in rows:
    if r['experiment'] == 'M1-count' and r['variant'] != 'plain':
        m = re.match(r'uniform-w(\d+)-p(\d+)-actual.*-n(\d+)',r['case'])
        w,pct,n=map(int,m.groups())
        sweeps[(r['threads'],n,pct,r['variant'])][w]=r
for (t,n,pct,v), ws in sorted(sweeps.items()):
    win=[]; overlap=[]; loss=[]
    for w,r in sorted(ws.items()):
        (win if r['ratio_ci95'][1]<1 else loss if r['ratio_ci95'][0]>1 else overlap).append(str(w))
    lines.append(f"| {t} | {n:,} | {pct}% | {v} | {', '.join(win) or 'none'} | {', '.join(overlap) or 'none'} | {', '.join(loss) or 'none'} |")
lines += ['', '## Encoded execution (M5)', '']
for r in rows:
    if r['experiment']=='M5' and r['variant']=='u8_codes':
        lo,hi=r['ratio_ci95']
        lines.append(f"- t{r['threads']}: u8 execution is {1/r['ratio_to_plain']:.2f}× the String-backed group throughput (95% CI [{1/hi:.2f}, {1/lo:.2f}]×), {r['ns_per_value']:.3f} ns/row, {r['items_per_second']/1e6:.2f} million rows/s. This combines compact keys with direct-address grouping; it is not solely a bandwidth-compression effect.")
lines += ['', '## Observed crossovers and contrary outcomes', '']
for t in (1,24):
    byte_cases = [v for (tt,n,pct,codec),v in sweeps.items() if tt==t and codec=='bytes']
    for_cases = [v for (tt,n,pct,codec),v in sweeps.items() if tt==t and codec=='for']
    if t==1 and all(all(v[w]['ratio_ci95'][1]<1 for w in (4,8,12,16)) and all(v[w]['ratio_ci95'][0]>1 for w in (20,24,32)) for v in byte_cases):
        lines.append('- **t1 count crossover:** byte offsets clearly win at 4–16 bits and clearly lose at 20–32 bits at both sizes and all selectivity targets. The measured crossover is bracketed between 16 and 20 bits, coinciding with the u16→u32 storage step.')
    if t==1 and all(all(r['ratio_ci95'][0]>1 for r in v.values()) for v in for_cases):
        lines.append('- **t1 FOR count:** no winning width was observed from 4 through 32 bits, despite fewer input bytes.')
    if t==24 and all(all(r['ratio_ci95'][1]<1 for r in v.values()) for v in byte_cases+for_cases):
        lines.append('- **t24 count:** bytes and FOR clearly beat plain at every tested width, size and target selectivity. No losing crossover is observed through 32 bits; results do not establish behavior above 32 bits.')
for t in (1,24):
    q=[r for r in rows if r['experiment']=='M1-Q6' and r['threads']==t and r['variant']=='bytes']
    lines.append(f"- **Q6 t{t}:** bytes/plain time ratios are "+', '.join(f"{r['ratio_to_plain']:.3f} at n={r['n']:,}" for r in q)+'. Byte truncation is the fastest median in each Q6 case.')
lines.append('- **Q6 versus count:** t1 FOR improves the real multi-column Q6 shape even though it loses every t1 single-column count sweep. [INFERENCE] The tradeoff depends on predicate work, column widths and late-materialized access, not merely a universal “decode ns/value.” The experiments do not isolate those contributions individually.')
for r in rows:
    if r['experiment']=='M3' and r['variant']!='plain':
        lo,hi=r['ratio_ci95']
        lines.append(f"- **M3 {r['case']} t{r['threads']}:** decode→merge→re-encode costs {r['ratio_to_plain']:.2f}× plain time (95% CI [{lo:.2f}, {hi:.2f}]). [INFERENCE] Input-byte savings do not repay decode, intermediate traffic and re-encoding in this implementation.")
for (exp,case,t),rs in groups.items():
    if exp=='M2':
        best=min(rs,key=lambda r:r['median_ns'])
        r64=next(r for r in rs if r['variant']=='plain_offset64')
        lo,hi=r64['ratio_ci95']
        lines.append(f"- **M2 {case} t{t}:** fastest median is {best['variant']} at {best['items_per_second']/1e6:.2f} Mprobes/s. Plain u64/u32-offset time ratio is {r64['ratio_to_plain']:.3f} [{lo:.3f}, {hi:.3f}]. "+('The offset CI overlaps 1; this run does not resolve an offset-speed winner.' if lo<=1<=hi else 'The offset CI excludes 1.'))
lines.append('- **M2 scope:** compressed payloads do not uniformly lose: byte payload/u32 offsets wins the t1 partkey→suppkey case, while several other differences are unresolved. [INFERENCE] Longer expansions and smaller payload working sets can offset decode cost. This refutes a universal “index runs must never be compressed” conclusion from these measurements alone.')
lines += ['', '## Interpretation and limits', '',
          '- The width table is the target-specific measured crossover evidence. A smaller resident representation alone is not evidence of a scan win; decoding, dictionary reconstruction, and cache behavior are included in the observed ratios.',
          '- [INFERENCE] Where compressed representations lose despite lower logical bytes, decode/control costs and/or lower parallel efficiency outweigh the saved input traffic. This study does not have hardware performance-counter attribution, so it cannot identify a unique microarchitectural cause.',
          '- [INFERENCE] Dictionary reconstruction may be especially unfavorable for high-cardinality uniform values because it introduces dependent dictionary reads. M5 avoids reconstruction by executing on codes; its comparison is intentionally a different question.',
          '- [INFERENCE] Differences between t1 and t24 or nonmonotone width outcomes can reflect the 64 MiB aggregate L3, two NUMA nodes, SMT, scheduling, and contention. Ratios with CIs overlapping 1 do not establish a winner.',
          '- M2 answers the payload/offset question with an unchanged directory and workload. M3 includes decode and re-encode, but excludes allocation/build costs. Neither result justifies a universal storage-role rule beyond the measured cases.', '',
          '## Complete measurements', '',
          '| Experiment / case | t | Variant | Median ns/value | 95% CI ns/value | Ratio to plain (95% CI) | Effective GB/s | M items/s | Resident MiB |',
          '|---|---:|---|---:|---|---|---:|---:|---:|']
for r in rows:
    lo,hi=r['ci95_ns'];rl,rh=r['ratio_ci95'];n=r['n']
    bw='n/a' if r['experiment']=='M2' else f"{r['effective_gbs']:.3f}"
    lines.append(f"| {r['experiment']}: {r['case']} | {r['threads']} | {r['variant']} | {r['ns_per_value']:.4f} | [{lo/n:.4f}, {hi/n:.4f}] | {r['ratio_to_plain']:.3f} [{rl:.3f}, {rh:.3f}] | {bw} | {r['items_per_second']/1e6:.3f} | {r['resident_bytes']/1048576:.3f} |")
lines += ['', '## Reproduction', '', '```sh',
          'python3 experiments/storage-bench/cpu/extract_strings.py',
          "nice -n 19 env CARGO_TARGET_DIR=/data/mini_linq/experiments/storage-bench/cpu/target RUSTFLAGS='-C target-cpu=native' cargo build --release -j1 --manifest-path experiments/storage-bench/cpu/Cargo.toml",
          'experiments/storage-bench/cpu/target/release/storage-cpu-bench --small',
          '# Only after obtaining an exclusive quiet CPU measurement window:',
          'experiments/storage-bench/cpu/target/release/storage-cpu-bench --machine-quiet',
          'python3 experiments/storage-bench/cpu/summarize.py', '```', '',
          '`--only M1|M2|M3|M5` selects an experiment; set `CPU_BENCH_OUTPUT` to a separate JSON path for a partial run. '
          'The supplemental `cpu-2026-10-28-m2-interleaved.json` preserves the controlled M2 rerun; the main JSON contains the final 476 rows and superseded M2 provenance.']
(root/'cpu-2026-10-28.md').write_text('\n'.join(lines)+'\n')
print(f'Rendered {len(rows)} observed rows to cpu-2026-10-28.md')
