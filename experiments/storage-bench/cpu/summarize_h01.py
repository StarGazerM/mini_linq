#!/usr/bin/env python3
"""Render measured M6 contrasts and actual uninstrumented-worker profiling."""
import json
import pathlib
import random
import statistics
root=pathlib.Path('/data/mini_linq/experiments/storage-bench/results')
p=root/'cpu-h01-decomposition-2026-10-29.json'
doc=json.loads(p.read_text())
rows={r['variant']:r for r in doc['rows']}
for c in doc['contrasts']:
    a=rows[c['from']]['samples_ns'];b=rows[c['to']]['samples_ns'];rng=random.Random(130129);diffs=[]
    for _ in range(2000):
        ids=[rng.randrange(len(a)) for _ in a]
        diffs.append(statistics.median([a[i] for i in ids])-statistics.median([b[i] for i in ids]))
    diffs.sort();c['difference_of_medians_ci95_ns']=[diffs[50],diffs[1950]]
profile_path=root/'cpu-h01-perf-2026-10-29.analysis.json'
profile=json.loads(profile_path.read_text())
doc['uninstrumented_profile']=profile
oracle=json.loads(pathlib.Path('/data/mini_linq/features/workloads/target/bench/runs/sf-1/retail_h01/default/oracle.json').read_text())
actual=[group.split('/')+[str(x) for x in values] for group,values in zip(doc['metadata']['group_order'],doc['metadata']['reference']) if values[2]]
assert sorted(actual)==sorted(oracle['rows']), 'M6 result differs from independent DuckDB oracle'
doc['metadata']['independent_duckdb_oracle_exact']=True
p.write_text(json.dumps(doc,indent=2)+'\n')
m=doc['metadata']
lines=['# CPU h01 decomposition — 2026-10-29','',
'## Scope and protocol','',
f"TPC-H SF1: {m['n']:,} resident rows, {m['qualifying_rows']:,} qualifying rows; one CPU thread. Shipdate ≤1998-09-02; group by returnflag/linestatus; sums of quantity and price in cents, and count. Existing numeric inputs are already cents: summing them implements SUM(value)×100 without a second multiplication.",'',
'Five warmups and 21 retained samples per variant, rotated 12-way interleaving. Every result and warmup matched **H**, the primary uninstrumented-engine replica, exactly. The input (orderkey, linenumber) sequence was independently verified strictly increasing before timing, proving DISTINCT redundant on this dataset. Input construction and answer checking are outside timing; set/map allocation, growth, insertion, output conversion and destruction are inside timing.','',
'Build: Rust release, `-C target-cpu=native`, thin LTO, one codegen unit; compiled `nice -n 19 -j1`. No thread affinity/NUMA binding. Environment snapshots include CPU/GPU clocks, driver, `uptime`, top CPU processes and GPU tenants before/after the matrix and after every round. Any observed GPU tenant or external process ≥20% lifetime-average CPU causes rejection/repetition. Boundary checks cannot exclude all transient contention.','',
'2,000 bootstrap resamples: time medians are resampled independently; ratios and differences of medians resample paired round indices. Contrasts overlap and **must not be summed into an additive percentage decomposition**. No measured-result-driven tuning was performed.','',
'## Corrected baseline and binary provenance','',
'**Actual row ownership:** `runtime/cq-storage/src/rows.rs:155–178` gives `Rows::Row: Copy` and tuples of column references. Thus the generated nine-tuple contains seven references and two scaled i64 values; `.clone()` copies those fields, not String heaps. Group keys are borrowed String pairs. H performs zero per-row String heap clones. J is a hypothetical owned-clone control, not engine behavior.',
f"Owned input row size is {m['row_bytes']} bytes excluding String allocations; borrowed nine-field dedup key size is {m['borrowed_key9_bytes']} bytes.",'',
'**The originally profiled binary is counter-instrumented.** SHA256 of `target/expanded-diff/build/current/release/tpch-derived-retail_h01` exactly matches the retained `89a0593ebef41215/out/tpch_derived_retail_h01` artifact. Its Cargo fingerprint is `features=["counters"]`, `rustflags=[]`. Under `counters`, CountingInner is DefaultHasher/SipHash, with thread-local instrumentation; it is not the uninstrumented direct-ahash implementation. The parent-supplied ~45% CountingHasher profile therefore does not explain uninstrumented execution.','',
'**H is primary:** current uninstrumented Dedup aliases direct per-set HashState/ahash. H uses that API with fixed stable keys for repeatability. B/C/D/J and L are relative to this direct-ahash baseline. A freezes the historical **counters-disabled fresh-RandomState wrapper bug**, now fixed by the parent; it is neither the current engine nor the actual counters-enabled profile. I retains only the write/finish wrapper with stable state. A is an artifact control, not a causal explanation of uninstrumented h01.','',
'The initial A-based timing attempt was cancelled when provenance changed the experiment baseline; no samples from that cancelled attempt are reported. The replacement matrix below measures all final variants together.','',
'## Variants and measurements','',
'| ID | Definition |','|---|---|',
'| H | Primary: borrowed nine-field dedup key, direct stable ahash, no reservation, borrowed String-pair SipHash grouping. |',
'| B | H with dedup capacity reserved for n input rows. |',
'| C | H with HashSet<&Row>: smaller table entry and seven-field hash rather than nine-field key. |',
'| D | H with u8 flag/status row fields; same borrowed-nine-field shape; array-indexed grouping. |',
'| E | No dedup; borrowed String-pair direct-ahash grouping. |',
'| F | No dedup; code row tuples; array grouping. |',
'| G | No dedup; separate code, quantity, price and date arrays. |',
'| A | Historical invalid fresh-RandomState-per-hash wrapper control; not an engine replica. |',
'| I | H with write/finish-only wrapper, stable keys identical to H. |',
'| J | H with owned nine-tuple dedup keys, two String heap clones per qualifying row. |',
'| K | No dedup; same String-pair SipHash grouping as H. |',
'| L | K plus one direct-ahash hash of each borrowed nine-tuple, consumed by black_box; no dedup table. |','',
'| Variant | Median ms | 95% CI ms | Ratio to H (95% CI) | Ratio to historical A | ns/input row |','|---|---:|---|---|---:|---:|']
for r in sorted(doc['rows'],key=lambda r:(not r['variant'].startswith('H_'),r['variant'])):
    lo,hi=r['ci95_ns'];rl,rh=r['ratio_to_H_ci95']
    lines.append(f"| {r['variant']} | {r['median_ns']/1e6:.3f} | [{lo/1e6:.3f}, {hi/1e6:.3f}] | {r['ratio_to_H']:.4f} [{rl:.4f}, {rh:.4f}] | {r['ratio_to_A']:.4f} | {r['ns_per_input_row']:.3f} |")
lines+=['','## Paired contrasts','','Positive savings mean the destination is faster. JSON also retains historical-A ratio CIs and median-of-paired-differences statistics.','','| Change | From → to | Saved median ms (95% CI) | To/from ratio (95% CI) |','|---|---|---|---|']
for c in doc['contrasts']:
    lo,hi=c['difference_of_medians_ci95_ns'];rl,rh=c['ratio_ci95']
    lines.append(f"| {c['name']} | {c['from'].split('_')[0]} → {c['to'].split('_')[0]} | {c['difference_of_medians_ns']/1e6:.3f} [{lo/1e6:.3f}, {hi/1e6:.3f}] | {c['ratio_to_from']:.4f} [{rl:.4f}, {rh:.4f}] |")
def ms(letter):return next(r['median_ns']/1e6 for n,r in rows.items() if n.startswith(letter+'_'))
h=ms('H')
lines+=['','## Attribution supported by the controlled experiments','',
f"- **Dedup removal (H−K): {h-ms('K'):.3f} ms**, {(h-ms('K'))/h*100:.1f}% of H. Includes hashing, probes, key storage, growth and destruction—not String heap cloning.",
f"- **Reservation/growth (H−B): {h-ms('B'):.3f} ms**, {(h-ms('B'))/h*100:.1f}% of H. Bundles allocations, entry migration and repeated rehashing; not allocation alone.",
f"- **One hash per qualifying row (L−K): {ms('L')-ms('K'):.3f} ms.** A hash-only probe, not the exact hash fraction of H, because H also rehashes during growth and incurs table/cache interactions.",
f"- **Wrapper API (I−H): {ms('I')-h:.3f} ms.** Contrary to an assumed wrapper penalty, the stable wrapper is slightly faster here. It retains the same seed keys, but byte-oriented versus specialized primitive writes can produce different hashes and table probe distributions. [INFERENCE] Hash distribution and code generation may explain the reversal; this experiment does not isolate them. Do not attribute the historical fresh-state control's large penalty to wrapping alone.",
f"- **Owned materialization control (J−H): {ms('J')-h:.3f} ms.** String clones/allocations plus larger owned table entries. H itself clones no String heaps; this contrast cannot explain real-engine clone cost.",
f"- **Grouping hasher (K−E): {ms('K')-ms('E'):.3f} ms**, dedup absent and String representation held constant.",
f"- **String/hashed grouping to codes/array (E−F): {ms('E')-ms('F'):.3f} ms.** Combines compact rows, avoiding string hashing and direct addressing; not pure string comparison cost.",
f"- **Columnar layout (F−G): {ms('F')-ms('G'):.3f} ms** after dedup removal and encoded grouping.",'',
'## Actual uninstrumented-worker perf profile','',
'Profiled the retained no-feature `cargo xtask bench` worker directly, not the top-level executable (which had subsequently been overwritten by a counters build). Full SHA256, Cargo fingerprint, exact command and environment are in the attached profile provenance JSON.',
f"- Binary: `{profile['provenance']['binary']}`.",
f"- SHA256: `{profile['provenance']['binary_sha256']}`.",
'- `perf record -g --call-graph dwarf,16384 -e cycles:u -e page-faults -F 199`; default query configuration, RAYON_NUM_THREADS=1, actual SF1 inputs and existing DuckDB oracle. One worker warmup plus 21 checked query runs. This is a diagnostic profile, not a substitute for the interleaved M6 samples.',
f"- Worker load_ms: {profile['worker_metrics'].get('load_ms')}; profiled query median_ms: {profile['worker_metrics'].get('median_ms')}; instrumented flag: {profile['worker_metrics'].get('instrumented')}.",
'- Whole-process capture includes load. Callchains classify samples as load, query, or other; only query-classified cycles form the query role table below. [INFERENCE] Symbol-based classification is conservative and may leave inlined/unwound/truncated samples unclassified. User-cycle samples cannot measure kernel page-fault handling time; page-fault events are reported separately.', '',
'### Phase shares (weighted sampled cycles)', '', '| Phase | Weighted cycle share | Samples |','|---|---:|---:|']
for phase,stats in profile['cycle_phases'].items():
    lines.append(f"| {phase} | {stats['percent']:.2f}% | {stats['samples']} |")
lines+=['','### Query-only self-symbol roles','','| Role | Weighted query-cycle share | Samples |','|---|---:|---:|']
for role,stats in profile['query_roles'].items():
    lines.append(f"| {role} | {stats['percent']:.2f}% | {stats['samples']} |")
lines+=['','Zero standalone allocation/copy or decimal self samples do **not** mean zero allocation/decimal cost: allocator work can be inlined or sampled in page-fault contexts, and scan/decimal/reduction code is fused. Page-fault-context cycles are contextual samples, not a measurement of total kernel page-fault time.']
lines+=['','### Top query self symbols','','| Symbol | Role | Weighted query-cycle share |','|---|---|---:|']
for row in profile['top_query_symbols']:
    lines.append(f"| `{row['symbol'].replace('|','/ ')}` | {row['role']} | {row['percent']:.2f}% |")
lines+=['','### Sampled page-fault events','','| Phase | Samples | Weighted event share |','|---|---:|---:|']
for phase,stats in profile['fault_phases'].items():
    lines.append(f"| {phase} | {stats['samples']} | {stats['percent']:.2f}% |")
lines.append(f"Dedup hashing with a `reserve_rehash` ancestor alone accounts for {profile['rehash_dedup_hash_query_cycle_percent']:.2f}% of query-classified weighted cycles. This is an inclusive context subdivision of the dedup-hashing self share, not an additional share.")
lines.append('This large rehash-context share and the H→B reservation contrast support growth/rehashing as a major cost. The much smaller L−K sequential hash-only contrast is not contradictory: table growth hashes keys reached through large, less-local table entries and also performs migration/probing. [INFERENCE] Locality and extra hash invocations explain why a sequential hash microprobe cannot stand in for in-table hash cost.')
lines+=['','## Relation to 2566 ms compiled versus 819 ms DuckDB','',
f"The supplied historical times imply a 3.13× gap; DuckDB was not rerun here. M6 measures primary H={h:.3f} ms and lower-bound G={ms('G'):.3f} ms. The actual retained no-feature worker was profiled separately above. Do not subtract microbenchmark contrasts directly from the historical 2566 ms: build flags/source revisions, seed choice, allocator/cache behavior and measurement scope differ.",'',
'[INFERENCE] The verified primary key permits dedup elimination, and code-domain grouping eliminates hashing work that compression alone cannot. The controlled contrasts quantify target-specific opportunities, not the exact end-to-end share of the historical engine–DuckDB gap. The counter-instrumented perf capture must not be used as a profile of uninstrumented execution.','',
'## Exact result and environment','','```json',json.dumps(m['reference'],indent=2),'```','',
'Group order: A/F, A/O, N/F, N/O, R/F, R/O. Triples: quantity cents, extendedprice cents, count. Empty groups are zeros. The complete nonempty result also matched the independent existing DuckDB SF1 oracle exactly.']
accepted=[w for w in doc['windows'] if w.get('accepted')]
lines.append(f"{len(accepted)} accepted matrix; {len(doc['windows'])-len(accepted)} rejected/busy records. Full round snapshots and rejected samples remain in JSON.")
for w in accepted:lines.append(f"Accepted UTC window: {w['before']['utc'].strip()} through {w['after']['utc'].strip()}.")
lines+=['','## Reproduction','','```sh','python3 experiments/storage-bench/cpu/extract_strings.py',"nice -n 19 env CARGO_TARGET_DIR=/data/mini_linq/experiments/storage-bench/cpu/target RUSTFLAGS='-C target-cpu=native' cargo build --release -j1 --manifest-path experiments/storage-bench/cpu/Cargo.toml --bin h01",'experiments/storage-bench/cpu/target/release/h01 --small','# Only in a quiet authorized window; do not overlap these runs:','experiments/storage-bench/cpu/target/release/h01 --machine-quiet','python3 experiments/storage-bench/cpu/profile_h01.py','python3 experiments/storage-bench/cpu/analyze_h01_profile.py','python3 experiments/storage-bench/cpu/summarize_h01.py','```']
if 'm6b' in doc:
    from summarize_h01b import render
    lines[2:2] = ['**Scope update:** M6 below retains historical no-dedup controls for decomposition, not a recommendation to assume source-key uniqueness. M6b at the end measures exact DISTINCT using a checked ordering fact or general packed sort-unique; those strategies do not assume uniqueness.', '']
    lines += ['', render(doc['m6b'])]
(root/'cpu-h01-decomposition-2026-10-29.md').write_text('\n'.join(lines)+'\n')
print('Rendered final M6:',len(rows),'variants; H primary; actual no-feature perf included')
