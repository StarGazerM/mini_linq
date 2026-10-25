#!/usr/bin/env python3
"""Render retained CPU/GPU/DuckDB metrics plus exact paired par t1/t24 profiles.

Optional par inputs: performance_cpu_par_t{1,24}_sf{0.1,1,10}.json.
Existing results remain available when par artifacts are missing. Common-set
summaries compare CPU-default, CPU-par t1/t24 and DuckDB t1/t24 only where all
five have completed exact numeric results; GPU common-set summaries additionally
require an exact GPU result. No timeout or failed correctness run is imputed.
"""
import csv
import json
import math
from pathlib import Path

OUT = Path(__file__).resolve().parent


def load(name):
    return json.loads((OUT / name).read_text())


def load_par(sf, threads):
    path = OUT / f'performance_cpu_par_t{threads}_sf{sf}.json'
    if not path.exists():
        return {}
    document = json.loads(path.read_text())
    assert document['sf'] == sf and document['configuration'] == 'par' and document['threads'] == threads
    return {r['query']: r for r in document['rows']}


def geometric_speedup(rows, numerator, denominator):
    return math.exp(sum(math.log(r[numerator] / r[denominator]) for r in rows) / len(rows)) if rows else None


def number(value):
    return '—' if value is None else f'{value:.3f}'


def status_text(row):
    if row['status'] == 'timeout':
        return f">{row['status_details']['worker_wall_limit_seconds']} s timeout"
    return row['status']


def table(headers, rows):
    return '\n'.join(['| ' + ' | '.join(headers) + ' |',
                      '|' + '|'.join(['---'] + ['---:'] * (len(headers) - 1)) + '|'] +
                     ['| ' + ' | '.join(map(str, row)) + ' |' for row in rows])


def main():
    gpu = {(r['sf'], r['query']): r for r in load('performance_gpu_summary.json')['results'] if r.get('exact')}
    comparisons, summaries, common_summaries, sections = [], [], [], []
    cpu_by_sf = {sf: load(f'performance_cpu_sf{sf}.json') for sf in [1, 10]}
    exact_by_threads = {threads: load_par(0.1, threads) for threads in [1, 24]}
    exactness = []
    for threads in [1, 24]:
        rows = list(exact_by_threads[threads].values())
        complete = [r for r in rows if r['status'] == 'complete' and r['correctness'] == 'ok']
        exactness.append([f'par t{threads}', len(complete), len(rows),
                          ', '.join(r['query'].removeprefix('retail_') + ': ' + status_text(r) for r in rows if r not in complete) or ('none' if rows else 'not collected')])
    exactness_matrix = []
    catalog = sorted({s['query'] for s in cpu_by_sf[1]['selections']},
                     key=lambda query: (not query.startswith('retail_h'), query))
    for query in catalog:
        cells = [query.removeprefix('retail_')]
        for threads in [1, 24]:
            result = exact_by_threads[threads].get(query)
            cells.append(f"{result['correctness']} / {status_text(result)}"
                         if result else 'unavailable / missing')
        exactness_matrix.append(cells)
    sections.append('## Parallel SF0.1 exactness\n\nOne untimed execution per query and thread count, compared as an exact normalized set against authored DuckDB SQL. This is correctness evidence, not timing data.\n\n'
                    + table(['Configuration', 'Exact completed', 'Rows collected (of 27)', 'Unavailable'], exactness)
                    + '\n\n### Per-query SF0.1 exactness matrix\n\nCells show correctness / collection status; missing evidence is explicitly unavailable.\n\n'
                    + table(['Query', 'CPU-par t1 correctness / status', 'CPU-par t24 correctness / status'], exactness_matrix))
    spotchecks = []
    for threads in [1, 24]:
        path = OUT / f'performance_cpu_default_spotcheck_t{threads}_sf0.1.json'
        if path.exists():
            document = json.loads(path.read_text())
            assert document['sf'] == 0.1 and document['configuration'] == 'default'
            spotchecks.extend([r['query'].removeprefix('retail_'), f'default t{threads}',
                               r['correctness'], status_text(r)] for r in document['rows'])
    sections.append('### Default SF0.1 exactness spotcheck\n\nFive untimed default-query checks use isolated evidence, never the retained CPU performance measurements.\n\n'
                    + (table(['Query', 'Configuration', 'Correctness', 'Status'], spotchecks)
                       if spotchecks else 'Not collected.'))
    for sf in [1, 10]:
        cpu = cpu_by_sf[sf]
        duck = {r['gpu_query']: r['raw_timing']['threads'] for r in load(f'performance_duck_trash_sf{sf}.json')['results']}
        par = {threads: load_par(sf, threads) for threads in [1, 24]}
        phase_rows, plain_rows = [], []
        for selection in sorted(cpu['selections'], key=lambda s: (not s['query'].startswith('retail_h'), s['query'])):
            query = selection['query'].removeprefix('retail_')
            g = gpu.get((sf, query))
            row = {'sf': sf, 'query': query, 'gpu_exact': bool(g)}
            row.update({f'gpu_{phase}_ms': g[f'{phase}_ms'] if g else None for phase in ['build', 'query', 'total']})
            phases = [query] + ([number(g[f'{p}_ms']) for p in ['build', 'query', 'total']] if g else ['CPU-only'] * 3)
            plain = [query]
            for label, key in [('le', 'best_le'), ('advised', 'advised'), ('default', 'default')]:
                c = selection[key]
                # No elapsed timeout is ever substituted for a timing.
                status = status_text(c) if c else '/'.join(sorted({status_text(r) for r in cpu['rows'] if r['query'] == selection['query'] and r['configuration'] in ['le', 'le_runs']}))
                row[f'cpu_{label}_configuration'] = c['configuration'] if c else None
                row[f'cpu_{label}_status'] = status
                for output, field in [('uninstrumented_total', 'uninstrumented_total_median_ms'), ('profile_build', 'profile_build_at_median_total_ms'), ('profile_query', 'profile_query_residual_at_median_total_ms'), ('profile_total', 'profile_total_median_ms')]:
                    row[f'cpu_{label}_{output}_ms'] = c[field] if c and c['correctness'] == 'ok' else None
                row[f'cpu_{label}_runs'] = c['runs'] if c else 0
                row[f'cpu_{label}_profile_runs'] = c['profile_runs'] if c else 0
                values = [row[f'cpu_{label}_profile_{p}_ms'] for p in ['build', 'query', 'total']]
                formatted = [number(v) for v in values]
                if values[2] is None:
                    formatted[2] = status
                elif label == 'le':
                    formatted[2] += f" ({c['configuration']})"
                phases.extend(formatted)
                value = row[f'cpu_{label}_uninstrumented_total_ms']
                plain.append(number(value) if value is not None else status)
            row['cpu_le_search_complete'] = selection['le_search_complete']
            for threads in [24, 1]:
                c = par[threads].get(selection['query'])
                exact = bool(c and c['correctness'] == 'ok')
                value = c['uninstrumented_total_median_ms'] if exact else None
                row[f'cpu_par_t{threads}_configuration'] = 'par'
                row[f'cpu_par_t{threads}_status'] = status_text(c) if c else 'missing'
                row[f'cpu_par_t{threads}_exact'] = exact
                row[f'cpu_par_t{threads}_uninstrumented_total_ms'] = value
                row[f'cpu_par_t{threads}_runs'] = c['runs'] if c else 0
                row[f'cpu_par_t{threads}_profile_runs'] = c.get('profile_runs', 0) if c else 0
                for output, field in [('build', 'profile_build_at_median_total_ms'),
                                      ('query', 'profile_query_residual_at_median_total_ms'),
                                      ('total', 'profile_total_median_ms')]:
                    row[f'cpu_par_t{threads}_profile_{output}_ms'] = c.get(field) if exact else None
                values = [row[f'cpu_par_t{threads}_profile_{p}_ms'] for p in ['build', 'query', 'total']]
                formatted = [number(v) for v in values]
                if values[2] is None:
                    formatted[2] = (row[f'cpu_par_t{threads}_status']
                                    if not c or c['status'] != 'complete' else 'profile unavailable')
                phases.extend(formatted)
                plain.append(number(value) if value is not None else row[f'cpu_par_t{threads}_status'])
            for threads in [1, 24]:
                value = duck[query][str(threads)]['median_seconds'] * 1000
                row[f'duckdb_t{threads}_ms'] = value
                phases.append(number(value))
                plain.append(number(value))
            comparisons.append(row)
            phase_rows.append(phases)
            plain_rows.append(plain)
        headers = ['Query', 'GPU build', 'GPU query', 'GPU total']
        for label in ['LE', 'advised', 'default']:
            headers.extend([f'CPU-{label} build', f'CPU-{label} query', f'CPU-{label} total'])
        for threads in [24, 1]:
            headers.extend([f'CPU-par t{threads} setup+build', f'CPU-par t{threads} final evaluation', f'CPU-par t{threads} total'])
        headers.extend(['DuckDB t1', 'DuckDB t24'])
        statuses = {status: sum(r['status'] == status for r in cpu['rows']) for status in sorted({r['status'] for r in cpu['rows']})}
        three = sum(r['runs'] == 3 or r['profile_runs'] == 3 for r in cpu['rows'])
        coverage = ', '.join(f'{n} {s}' for s, n in statuses.items())
        par_coverage = '; '.join(
            f"par t{threads}: {sum(r['correctness'] == 'ok' and r.get('uninstrumented_total_median_ms') is not None for r in par[threads].values())} exact plain, "
            f"{sum(r.get('profile_total_median_ms') is not None for r in par[threads].values())} paired profiles, "
            f"{sum(r['status'] == 'timeout' for r in par[threads].values())} configuration-budget timeouts"
            for threads in [24, 1])
        sections.append(
            f"## SF{sf}: phase comparison\n\nAll values are milliseconds. CPU phase columns are paired **instrumented profile** measurements; GPU and DuckDB are ordinary timings. Parallel **setup+build / final evaluation** are not the same accounting as default/LE/advised exclusive build / query residual. Final evaluation includes boundary/teardown residual. **Uninstrumented whole-query totals are primary** for speedups. CPU-LE is selected by the minimum plain median of `le` and `le_runs`.\n\n"
            + table(headers, phase_rows)
            + f"\n\nCoverage: {len(cpu['rows'])} retained sequential configurations across 27 queries; {coverage}; {three} configurations used three runs in at least one completed mode. Parallel coverage (of 27 each): {par_coverage}.\n\n### SF{sf} uninstrumented CPU totals\n\nThese whole-query medians include construction and determine primary CPU speedups. Completed exact plain results remain shown when a later profile/counting mode times out.\n\n"
            + table(['Query', 'CPU-LE total', 'CPU-advised total', 'CPU-default total', 'CPU-par t24 total', 'CPU-par t1 total', 'DuckDB t1', 'DuckDB t24'], plain_rows)
        )
        primary_fields = ['cpu_default_uninstrumented_total_ms', 'cpu_par_t1_uninstrumented_total_ms', 'cpu_par_t24_uninstrumented_total_ms', 'duckdb_t1_ms', 'duckdb_t24_ms']
        fields = ['duckdb_t1_ms', 'duckdb_t24_ms'] + [f'cpu_par_t{threads}_{mode}_total_ms' for threads in [24, 1] for mode in ['uninstrumented', 'profile']] + [f'cpu_{label}_{mode}_total_ms' for mode in ['uninstrumented', 'profile'] for label in ['le', 'advised', 'default']]
        for field in fields:
            eligible = [r for r in comparisons if r['sf'] == sf and r['gpu_exact'] and r[field] is not None and r[field] > 0]
            summaries.append({'sf': sf, 'comparator': field, 'queries': [r['query'] for r in eligible], 'n': len(eligible), 'geometric_mean_speedup': math.exp(sum(math.log(r[field] / r['gpu_total_ms']) for r in eligible) / len(eligible)) if eligible else None})
        common = [r for r in comparisons if r['sf'] == sf and all(r[f] is not None and r[f] > 0 for f in primary_fields)]
        for denominator in ['cpu_par_t1_uninstrumented_total_ms', 'cpu_par_t24_uninstrumented_total_ms']:
            for numerator in primary_fields:
                if numerator == denominator:
                    continue
                common_summaries.append({
                    'sf': sf, 'set': 'cpu_default_par_t1_par_t24_duckdb_t1_t24',
                    'numerator': numerator, 'denominator': denominator,
                    'queries': [r['query'] for r in common], 'n': len(common),
                    'geometric_mean_speedup': geometric_speedup(common, numerator, denominator),
                })
        common_gpu = [r for r in common if r['gpu_exact'] and r['gpu_total_ms'] > 0]
        for numerator in primary_fields:
            common_summaries.append({
                'sf': sf, 'set': 'gpu_cpu_default_par_t1_par_t24_duckdb_t1_t24',
                'numerator': numerator, 'denominator': 'gpu_total_ms',
                'queries': [r['query'] for r in common_gpu], 'n': len(common_gpu),
                'geometric_mean_speedup': geometric_speedup(common_gpu, numerator, 'gpu_total_ms'),
            })
    with (OUT / 'performance_comparison.tsv').open('w') as stream:
        writer = csv.DictWriter(stream, fieldnames=list(comparisons[0]), delimiter='\t')
        writer.writeheader()
        writer.writerows(comparisons)
    (OUT / 'performance_speedups.json').write_text(json.dumps({
        'formula': 'exp(mean(log(comparator_total_ms / gpu_total_ms)))',
        'selection': 'CPU-LE minimizes exact uninstrumented median of le/le_runs; par is fixed par. Unavailable timings are excluded, never imputed.',
        'results': summaries,
        'common_formula': 'exp(mean(log(numerator_ms / denominator_ms))); greater than one favors denominator',
        'common_selection': 'Shared exact completed positive timings for CPU-default, par t1/t24 and DuckDB t1/t24; GPU sets also require exact completed GPU. Query lists identify every denominator set.',
        'common_results': common_summaries,
    }, indent=2) + '\n')
    means = table(['Scale', 'Comparator', 'Common exact queries (n)', 'Geometric-mean GPU speedup'], [[f"SF{r['sf']}", r['comparator'], r['n'], number(r['geometric_mean_speedup']) + '×'] for r in summaries])
    common_means = table(['Scale', 'Common set', 'Numerator', 'Denominator (favored above 1×)', 'n', 'Geometric-mean speedup'],
                         [[f"SF{r['sf']}", r['set'], r['numerator'], r['denominator'], r['n'],
                           number(r['geometric_mean_speedup']) + '×'] for r in common_summaries])
    intro = '''# CQ TPC-H-shaped performance comparison

## Scope and methodology

These are the **27 authored CQ-shaped catalog queries**, not full standard TPC-H coverage: 26 GPU-suite queries plus **h14, CPU-only**, at SF1 and SF10. GPU name `hXX`/`dXX` maps to CPU catalog name `retail_hXX`/`retail_dXX`. Numerical results require exact answers; no timeout is interpreted as an execution time. All times are milliseconds, lower is better.

Hardware: **AMD Ryzen Threadripper PRO 5945WX, 12 cores / 24 hardware threads**, approximately **125.77 GiB RAM**, **NVIDIA RTX 6000 Ada Generation (49,140 MiB)**, driver **580.178.04**. GPU compilation targeted `sm_89` with CUDA 12.9 (NVIDIA HPC SDK 25.5). See [hardware](performance_hardware.json). Retained CPU-default/LE/advised workers are sequential; CPU-par explicitly uses Rayon t1/t24. Run timed CPU windows serially, with no concurrent compilation.

### GPU: statistics-derived advised plans

GPU uses the checked `--mode advised` plans, one warmup and five measured executions. All 52 included GPU results passed exact comparison. SF1 additionally checks sequential-program conformance. The retained results are [the original suite](performance_gpu.json), [fresh-process SF10 recovery for d27/d72_inner/d85](performance_gpu_sf10_remaining.json), and **h19 from [the exact interning suite](gpu_tpch_intern.results.json)**. Initial device allocation failures are not timings. h14's prefix predicate requires order-preserving interning and has no GPU result.

- **Build:** CUDA-event elapsed construction, selection/index preparation and scheduled work. Staged plans also charge host-graded stages and nullable intermediate host publication to build; it is not exclusively device index construction.
- **Query:** median of each sample's kernels + dedup. Dedup includes grouping/reduction, canonicalization and device publication, not merely final DISTINCT.
- **Total:** median total. These independent medians need not sum exactly.
- Input file loading, upload/compression, module initialization, final answer download/decoding and comparison are excluded. Query-required intermediate publication is included. These are resident-input execution measurements, not end-to-end latency.
- h02 stage 1 and h15 stage 2 execute on the host after checked device rejection; these hybrid measurements are not wholly device execution.

H19 uses author-controlled `u32` dictionaries for shipping mode/instruction, brand and container. CPU and GPU use matching sorted-distinct dictionaries; missing literals use `u32::MAX`. Input dictionary preparation is excluded from timing. See [statistics derivation](tpch_statistics.md) and [`benchmark.rs`](../tpch/src/benchmark.rs).

### CPU: LE, advised and default

Each of the 27 queries has **default, le, le_runs and advised** measured independently at each scale. There is **no family pruning**. CPU-default is exactly `default`; CPU-advised is exactly `advised`; CPU-LE is the fastest successfully measured `le`/`le_runs` by its uninstrumented whole-query median. If a candidate times out, the best completed candidate is reported and `cpu_le_search_complete` records the incomplete search. No different planner substitutes for an unavailable LE result.

The original policy was one warm execution, **five plain runs**, one independent counter execution, and **five profile runs**, reducing to **three** timed runs when a warm or timed execution exceeded **60 seconds**, with a **600-second combined wall budget per configuration**. All SF1 and the first part of SF10 used that policy. **At the user's priority override, the remaining SF10 work switched to a 100-second cap, three runs above 10 seconds, and profiles only for each column's selected default / best-LE / advised configuration.** Plain runs still cover every configuration; already completed results are retained, including earlier profiles for nonwinning LE candidates. Per-row policy metadata distinguishes the two collection policies. Completed modes are retained across interruption; incomplete work resumes under the new cap.

Caps include loading, warmup, measurements and exact comparisons across modes. `>100 s timeout` (or the earlier `>600 s timeout`) is a **configuration-budget status, not a numeric query timing or per-execution lower bound**. A completed exact plain measurement remains usable if a later profile phase times out; missing phases stay unavailable.

**Plain total** surrounds `query::run`, including query-local preprocessing and structure/index construction. Parquet loading, load-time zone preparation, canonicalization, sorting, serialization, exact comparison and final answer destruction are excluded. Every warm and timed answer is compared as a normalized exact set against the same authored DuckDB SQL at the same scale. Only DECIMAL trailing-zero formatting is normalized; there is no floating tolerance.

**Profile build** is `BUILD_EXCLUSIVE_TOTAL`: the sum of exclusive elapsed construction time (elapsed minus separately measured source production, saturating) for operator kinds ending `_BUILD`. **Profile query** is the paired profile total minus that build; **profile total** is the median-total profile execution. The build tree rounds to 0.001 ms. All three refer to the same execution, not independent phase medians. Profiling materializes build inputs and adds clocks/bookkeeping, so its timings can differ substantially from plain execution. **Never subtract profile build from plain total.** Zero build means no time in these explicit scopes, not no allocation or hashing.

The separate counter run supplies checked operator rows; profiles check exact answers, graph labels, rows and the top-level clock. Independent primitive insertion-count agreement is recorded as `primitive_build_counts_verified`, separately from exactness. Per-configuration plain/profile samples, run counts, statuses and profiles are retained in the raw SF-specific directories and consolidated JSON/TSV. SF1 and SF10 data/cache directories are isolated; compiled artifacts may be reused only when source fingerprints agree.

Boundaries: [`worker.rs`](../../../features/workloads/xtask/src/tpch/worker.rs), [collector](performance_cpu_collect.py). Source hashes and machine metadata: [SF1](performance_cpu_environment_sf1.json), [SF10](performance_cpu_environment_sf10.json).

### CPU: fixed parallel configuration

CPU-par always selects the authored **`par`** configuration, never the best of multiple plans. Each scale and thread count has isolated measurement artifacts: `RAYON_NUM_THREADS=1` or `24`, raw data under `performance_cpu_raw/par/t{threads}/sf{sf}`, consolidated `performance_cpu_par_t{threads}_sf{sf}.json` and `.tsv`. Scale/thread-independent timing, counting and profile binaries and Cargo output are shared under `/tmp/mini-linq-performance-cpu/par`; exported data remains scale-specific. Cache stamps hash compiler/runtime Rust sources and existing Cargo/toolchain inputs, including Tasks, pull, IR, planner, printer, storage and facade. Existing raw results must match their source stamps before reuse; stale evidence is rejected rather than silently overwritten. Generated consumers depend directly on Rayon because emitted task code uses `::rayon`.

SF0.1 runs one untimed exact comparison for every query at each thread count. A separate `--default-spotcheck` checks h01, h02, h06, h13 and h19 by default, with isolated `performance_cpu_default_spotcheck_t{threads}_sf0.1` artifacts; it never changes the existing CPU measurements. SF1/SF10 run one exact warmup and five plain timed runs, an independent untimed counting run, then one exact profile warmup and five profile timed runs. Any warmup or timed query above **10 seconds** reduces that timed mode to the first **three** samples (including when the fourth/fifth sample triggers reduction). Every result is checked against the same authored SQL with exact normalized-set comparison. The **100-second combined configuration wall cap** includes all worker loading, warmup, measurement and comparisons across plain/counts/profile. Completed exact plain metrics remain valid if counting or profiling subsequently times out; unavailable phases stay null, never estimated. Query-local task scheduling and construction are included in totals; input loading and answer comparison are excluded. Parallel environment, source hashes and command logs are retained separately from the original CPU evidence.

Parallel **setup+build** uses `TASKS_BUILD_WALL_TOTAL`, the measured host-wall **union** of generated setup/publication intervals across all Tasks programs during that same query execution. Nested/concurrent build intervals count once, never once per task. Setup+build includes upstream scans, partition routing and joins needed to publish stores, not just exclusive index insertion. **Final evaluation** is the same median profile total minus that measured union, including boundary/teardown overhead. Per-program `TASKS_QUERY_WALL` final evaluation/collection spans remain raw diagnostics, not an additive wall split. Task-local/lazy construction during final evaluation remains there unless it enters an explicit build interval. Ordinary operator spans can overlap across tasks and exceed root wall time; they are **never summed** for the parallel wall split. **Parallel setup+build is not identical to default/LE/advised exclusive build** (`BUILD_EXCLUSIVE_TOTAL`). Internal JSON fields retain `profile_build` / `profile_query_residual` naming, with these parallel-specific definitions. Phase values never split an uninstrumented total, and no missing phase is inferred from plain timing. **Uninstrumented whole-query totals remain the primary comparison.**



### DuckDB

DuckDB **v1.4.4 (Andium), `6ddac802ff`**, explicit t1/t24. CLI `.timer` measures complete authored SQL in **`.mode trash`**, with one warmup and five samples, avoiding answer CSV formatting/output and comparison. Intrinsic result materialization/consumption remains timed; resolution is 1 ms. Database generation/export, process opening and thread configuration are excluded. Scanning, reads/decompression, hash construction, aggregation and DISTINCT are included. There is no separate external index-build phase. The report uses [SF1](performance_duck_trash_sf1.json) and [SF10](performance_duck_trash_sf10.json), not CSV-output timers.

## Reproduction

From the repository root (run CPU scales serially):

```sh
python3 experiments/gpu/compare/performance_cpu_collect.py all --sf 1 --runs 5 --cap-seconds 600
# Initial SF10 policy; completed results are retained:
python3 experiments/gpu/compare/performance_cpu_collect.py all --sf 10 --runs 5 --cap-seconds 600
# Resume remaining SF10 work using the user-authorized final policy:
python3 experiments/gpu/compare/performance_cpu_collect.py build --sf 10 --cap-seconds 100 --slow-seconds 10 --profile-winners-only
python3 experiments/gpu/compare/performance_cpu_collect.py measure --sf 10 --cap-seconds 100 --slow-seconds 10 --profile-winners-only
python3 experiments/gpu/compare/performance_cpu_collect.py consolidate --sf 10 --cap-seconds 100 --slow-seconds 10 --profile-winners-only
python3 experiments/gpu/compare/performance_cpu_collect.py cleanup --sf 10
# Parallel protocol: complete SF0.1 exactness first, then time SF1/SF10.
# Run serially; do not overlap builds or measurements:
for threads in 24 1; do
  python3 experiments/gpu/compare/performance_cpu_collect.py all --par --threads "$threads" --sf 0.1 --cap-seconds 100 \
    --database features/workloads/target/bench/data/tpch-0.1/dbgen.duckdb
done
python3 experiments/gpu/compare/performance_cpu_collect.py all --default-spotcheck --sf 0.1 --cap-seconds 100 \
  --database features/workloads/target/bench/data/tpch-0.1/dbgen.duckdb
for sf in 1 10; do
  for threads in 24 1; do
    python3 experiments/gpu/compare/performance_cpu_collect.py all --par --threads "$threads" --sf "$sf" --runs 5 --cap-seconds 100
  done
done
python3 experiments/gpu/compare/performance_report.py
```

`--database PATH` explicitly overrides the oracle/export database and records its resolved path in environment metadata; otherwise the existing `/data/tpch-gpu/sf{sf}/dbgen.duckdb` convention is unchanged. The SF0.1 commands assume the xtask-generated database already exists. For an initial parallel smoke subset, add repeatable `--query retail_hXX`; consolidation still preserves every catalog row, marking uncollected queries missing. This does not replace full 27-query exactness. `--default-spotcheck` accepts exactly five distinct `--query` overrides and defaults to one thread. Freeze compiler/runtime and worker sources before collecting; run every command serially, without concurrent builds or measurements.

The collector also exposes `adaptive`, `build`, `prepare`, `measure`, `consolidate`, and `cleanup` phases. Cleanup removes duplicate answers/oracles after exact checks while preserving measurement evidence; prepare regenerates the oracles. GPU reproduction (inside `experiments/gpu`):

```sh
CUDA_HOME=/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/cuda/12.9 CARGO_BUILD_JOBS=1 cargo oxide build --arch sm_89 -- --release -p cq-gpu-tpch
./target/release/cq-gpu-tpch compare/performance_gpu.json --mode advised
./target/release/cq-gpu-tpch compare/performance_gpu_sf10_remaining.json --mode advised --scale sf10 --query d27 --query d72_inner --query d85
./target/release/cq-gpu-tpch compare/gpu_tpch_intern.results.json --mode advised
```

DuckDB reproduction: `python3 experiments/gpu/compare/performance_duck.py`.
'''
    conclusion = '''## Geometric-mean speedups

Speedup = comparator total / GPU total; **greater than 1× favors GPU**. The geometric mean is `exp(mean(log(comparator_ms / gpu_ms)))`, equally weighting queries. The denominator counts common exact GPU queries with a completed numeric comparator measurement. h14 is excluded, as are unavailable/timed-out measurements; a timeout's elapsed wall time is never included. Different denominator sets are not directly comparable aggregate rankings. **Uninstrumented CPU comparisons are primary**; profile comparisons only describe the phase-table measurements.

'''
    common_conclusion = '''## Common-set parallel speedups

These summaries use the **same completed exact query set for CPU-default, CPU-par t1/t24 and DuckDB t1/t24** at each scale. GPU common-set summaries intersect that set with completed exact GPU results. h14 may participate in CPU/DuckDB comparisons but never GPU comparisons. Missing, failed, timed-out and zero-resolution timings are excluded from geometric means; no capped wall duration is substituted. Each JSON summary retains its query list. Speedup is numerator / denominator, so **greater than 1× favors the named denominator**. The historical pairwise GPU summaries above remain available and retain their original CPU metrics.

'''
    slower = []
    for sf in [1, 10]:
        group = [r for r in comparisons if r['sf'] == sf and r['gpu_exact']]
        for field in ['duckdb_t1_ms', 'duckdb_t24_ms', 'cpu_le_uninstrumented_total_ms', 'cpu_advised_uninstrumented_total_ms', 'cpu_default_uninstrumented_total_ms', 'cpu_par_t1_uninstrumented_total_ms', 'cpu_par_t24_uninstrumented_total_ms']:
            names = [r['query'] for r in group if r[field] is not None and r[field] < r['gpu_total_ms']]
            slower.append(f"- SF{sf}, {field}: " + (', '.join(names) if names else 'none among completed measurements') + '.')
    report = intro + '\n\n'.join(sections) + '\n\n' + conclusion + means + '\n\n' + common_conclusion + common_means + '\n\n## Queries where GPU is slower\n\n' + '\n'.join(slower) + '''

h02 includes host fallback/intermediate publication; h11 and h01 spend substantial GPU time in grouping/reduction/publication. These phase allocations are not hardware-counter diagnoses: occupancy, bandwidth and cache misses are not inferred. Differences near DuckDB's 1 ms timer resolution should not be treated as precise rankings.

Machine-readable outputs: [comparison TSV](performance_comparison.tsv), [speedups JSON](performance_speedups.json), [CPU SF1](performance_cpu_sf1.json), [CPU SF10](performance_cpu_sf10.json), [GPU summary](performance_gpu_summary.json). Retained raw CPU measurement artifacts are under `performance_cpu_raw/sf1` and `performance_cpu_raw/sf10`. Parallel JSON/TSV inputs are `performance_cpu_par_t{1,24}_sf{0.1,1,10}.{json,tsv}`, with per-thread raw artifacts under `performance_cpu_raw/par/t{1,24}/sf{0.1,1,10}`. Speedups JSON retains historical `results` and adds `common_results` with explicit numerator, denominator, query lists and counts.
'''
    (OUT / 'performance_report.md').write_text(report)
    print(means)


if __name__ == '__main__':
    main()
