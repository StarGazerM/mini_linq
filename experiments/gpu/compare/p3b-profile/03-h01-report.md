# CPU TPC-H retail_h01 default, SF1

> Point-in-time measurement: M0 timing worker, 2026-10-31 (source revision not stated in this report; preserved by `1031adf6`, 2026-10-31). **Superseded by:** the P4 CPU compiler-selection measurements in [PLANNING-REDESIGN](../../../../doc/PLANNING-REDESIGN.md); the wide-sort diagnosis below describes that worker, not the current selected storage.

Profiled the **already-built, uninstrumented timing worker**, not Cargo/xtask or a counting worker. No compilation. Command:

```text
/usr/bin/perf record -F 499 -g --call-graph dwarf,16384 \
  -o /data/mini_linq-wt/p3b-profile/h01.perf.data -- \
  /data/mini_linq-wt/m0-baseline/features/workloads/target/bench/crates/retail_h01/timing-worker \
  /data/mini_linq-wt/m0-baseline/features/workloads/target/bench/data/tpch-1/retail_h01 \
  default 9 \
  /data/mini_linq-wt/p3b-profile/h01.answers.json \
  /data/mini_linq-wt/p3b-profile/h01.metrics.json \
  /data/mini_linq-wt/m0-baseline/features/workloads/target/bench/runs/sf-1/retail_h01/default/oracle.json
```

The worker loads once, checks one warm execution against the cached DuckDB oracle, and performs nine measured executions. Evidence: 10,868 samples, **zero lost samples**, 23.41 s process wall; query median **1925.655 ms**, sample range 1896.591–2353.351 ms; load 1645.871 ms; four answers; peak RSS 1,323,060 KiB. `instrumented=false` in `h01.metrics.json`.

## Top symbols (self cycles, whole worker process)

| Symbol, shortened from Rust mangling | Self share | Interpretation |
|---|---:|---|
| `core::slice::sort::unstable::quicksort::quicksort<wide tuple>` | **57.78%** | Comparison sorting the pre-group valuation |
| `hashbrown::rustc_entry::HashMap::rustc_entry<(StringCode,StringCode),...>` | **14.15%** | Per-row hash group lookup/update |
| `alloc::vec::Vec<wide tuple>::dedup_by` | **3.78%** | Adjacent duplicate removal after sort |
| `core::slice::sort::shared::smallsort::small_sort_general<wide tuple>` | **2.61%** | Small partitions of the same canonicalization sort |
| `alloc::vec::spec_from_iter::SpecFromIter::from_iter<wide tuple>` | **1.68%** | Collecting the intermediate valuation stream |
| `clear_page_erms` | **1.27%** | Page initialization, process-wide |
| `cq_storage::dedup::Sorted::next` | **1.21%** | Canonicalization barrier / iterator overhead |
| `__memmove_avx_unaligned_erms` | **0.66%** | Copy/move, process-wide |
| `query::default::run` | **0.56%** | Non-inlined query body |
| scalar `quicksort` | **0.54%** | Other process sort work |
| `do_anonymous_page` / `_int_malloc` | **0.49% each** | Allocation/page faults, process-wide |
| `__memcmp_avx2_movbe` / `malloc_consolidate` | **0.48% each** | Process-wide compare/allocation |
| `StringCode::intern` | **0.46%** | Load-time interning, not query string cloning |
| sort `pivot::median3_rec<wide tuple>` | **0.39%** | Pivot selection |

Raw self-symbol table: `h01.symbols.txt`; call graphs: `h01.perf-report.txt`; raw record: `h01.perf.data`. Mangled names are preserved in the artifacts. Some shared optimized helper symbols carry `radix_retained` in their name although the worker was invoked with `default`; this is not evidence that another configuration ran. [INFERENCE] Compiler deduplication/aliasing accounts for those names.

## Three biggest attributable costs / why

1. **Whole-input comparison sort before grouping: at least 60.78% self cycles** (quicksort + smallsort + pivot). `runtime/cq-storage/src/dedup.rs:74–79` collects the stream, calls `sort_unstable_by`, then `dedup_by` before yielding anything. The sampled comparator tuple includes line identity, dictionary-coded flags/status, quantity/price-derived fields and date—not merely the two group keys. [INFERENCE] Sorting/materializing almost all qualifying lineitems, rather than a direct four-group streaming sum, explains most of the disparity against the grouping microbenchmark. The SQL explicitly selects DISTINCT projected lineitems (`features/workloads/src/queries/retail/tpch/retail_h01.rs:17–23`), while the authored query also supplies line identity evidence (`:57–70`); the sampled default path still pays this canonicalization barrier.
2. **Hash group entry/update: 14.15% self cycles.** The generated grouped fold creates a table and calls `.entry(keys).or_insert_with(...)` for every canonical row (`passes/cq-physical-realization/src/operators.rs:150–157`). Only four groups survive; [INFERENCE] repeated hashing/probing is disproportionately expensive compared with a specialized small-domain accumulator, even though the keys are already numeric dictionary codes.
3. **Adjacent-unique pass: 3.78% self cycles**, plus intermediate collection at 1.68% and `Sorted::next` at 1.21%. This is a second full pass over the wide materialized valuation after sorting (`dedup.rs:76–79`), with allocation/page traffic visible separately. These are self percentages, so they do not double-count the quicksort descendants.

## Boundary and interpretation

Perf covers the whole process: load, warm query, nine measured queries, correctness conversion/comparison and result drops. Percentages are not strictly query-only; load is ~1.65 s of the 23.41 s process. The worker's reported median excludes load, canonical answer comparison and result drop (`features/workloads/xtask/src/tpch/worker.rs:45–67,118–130`). No attempt is made to multiply process-wide cycle percentages by the query median as if they were exact query timings.

This M0 source and the sampled symbols use **`StringCode`**, not heap-allocated String group keys. Query text decoding happens only for four final answers (`retail_h01.rs:1–2,25,72–80`). The claim that string cloning/hashing dominates h01 is therefore unsupported by this profile; wide-row DISTINCT sorting dominates instead. The user-provided ~140x microbenchmark comparison was not rerun and has a different execution boundary/algorithm; no new ratio is claimed.

All source paths above are relative to `/data/mini_linq-wt/m0-baseline/`.
