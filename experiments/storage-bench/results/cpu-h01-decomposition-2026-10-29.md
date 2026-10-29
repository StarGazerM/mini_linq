# CPU h01 decomposition — 2026-10-29

**Scope update:** M6 below retains historical no-dedup controls for decomposition, not a recommendation to assume source-key uniqueness. M6b at the end measures exact DISTINCT using a checked ordering fact or general packed sort-unique; those strategies do not assume uniqueness.

## Scope and protocol

TPC-H SF1: 6,001,215 resident rows, 5,916,591 qualifying rows; one CPU thread. Shipdate ≤1998-09-02; group by returnflag/linestatus; sums of quantity and price in cents, and count. Existing numeric inputs are already cents: summing them implements SUM(value)×100 without a second multiplication.

Five warmups and 21 retained samples per variant, rotated 12-way interleaving. Every result and warmup matched **H**, the primary uninstrumented-engine replica, exactly. The input (orderkey, linenumber) sequence was independently verified strictly increasing before timing, proving DISTINCT redundant on this dataset. Input construction and answer checking are outside timing; set/map allocation, growth, insertion, output conversion and destruction are inside timing.

Build: Rust release, `-C target-cpu=native`, thin LTO, one codegen unit; compiled `nice -n 19 -j1`. No thread affinity/NUMA binding. Environment snapshots include CPU/GPU clocks, driver, `uptime`, top CPU processes and GPU tenants before/after the matrix and after every round. Any observed GPU tenant or external process ≥20% lifetime-average CPU causes rejection/repetition. Boundary checks cannot exclude all transient contention.

2,000 bootstrap resamples: time medians are resampled independently; ratios and differences of medians resample paired round indices. Contrasts overlap and **must not be summed into an additive percentage decomposition**. No measured-result-driven tuning was performed.

## Corrected baseline and binary provenance

**Actual row ownership:** `runtime/cq-storage/src/rows.rs:155–178` gives `Rows::Row: Copy` and tuples of column references. Thus the generated nine-tuple contains seven references and two scaled i64 values; `.clone()` copies those fields, not String heaps. Group keys are borrowed String pairs. H performs zero per-row String heap clones. J is a hypothetical owned-clone control, not engine behavior.
Owned input row size is 88 bytes excluding String allocations; borrowed nine-field dedup key size is 72 bytes.

**The originally profiled binary is counter-instrumented.** SHA256 of `target/expanded-diff/build/current/release/tpch-derived-retail_h01` exactly matches the retained `89a0593ebef41215/out/tpch_derived_retail_h01` artifact. Its Cargo fingerprint is `features=["counters"]`, `rustflags=[]`. Under `counters`, CountingInner is DefaultHasher/SipHash, with thread-local instrumentation; it is not the uninstrumented direct-ahash implementation. The parent-supplied ~45% CountingHasher profile therefore does not explain uninstrumented execution.

**H is primary:** current uninstrumented Dedup aliases direct per-set HashState/ahash. H uses that API with fixed stable keys for repeatability. B/C/D/J and L are relative to this direct-ahash baseline. A freezes the historical **counters-disabled fresh-RandomState wrapper bug**, now fixed by the parent; it is neither the current engine nor the actual counters-enabled profile. I retains only the write/finish wrapper with stable state. A is an artifact control, not a causal explanation of uninstrumented h01.

The initial A-based timing attempt was cancelled when provenance changed the experiment baseline; no samples from that cancelled attempt are reported. The replacement matrix below measures all final variants together.

## Variants and measurements

| ID | Definition |
|---|---|
| H | Primary: borrowed nine-field dedup key, direct stable ahash, no reservation, borrowed String-pair SipHash grouping. |
| B | H with dedup capacity reserved for n input rows. |
| C | H with HashSet<&Row>: smaller table entry and seven-field hash rather than nine-field key. |
| D | H with u8 flag/status row fields; same borrowed-nine-field shape; array-indexed grouping. |
| E | No dedup; borrowed String-pair direct-ahash grouping. |
| F | No dedup; code row tuples; array grouping. |
| G | No dedup; separate code, quantity, price and date arrays. |
| A | Historical invalid fresh-RandomState-per-hash wrapper control; not an engine replica. |
| I | H with write/finish-only wrapper, stable keys identical to H. |
| J | H with owned nine-tuple dedup keys, two String heap clones per qualifying row. |
| K | No dedup; same String-pair SipHash grouping as H. |
| L | K plus one direct-ahash hash of each borrowed nine-tuple, consumed by black_box; no dedup table. |

| Variant | Median ms | 95% CI ms | Ratio to H (95% CI) | Ratio to historical A | ns/input row |
|---|---:|---|---|---:|---:|
| H_engine_direct_stable_ahash | 2542.212 | [2536.699, 2563.560] | 1.0000 [1.0000, 1.0000] | 0.6819 | 423.616 |
| A_legacy_fresh_ahash_wrapper | 3728.191 | [3694.869, 3739.325] | 1.4665 [1.4431, 1.4703] | 1.0000 | 621.239 |
| B_H_reserved | 1349.806 | [1344.342, 1374.574] | 0.5310 [0.5272, 0.5387] | 0.3621 | 224.922 |
| C_borrowed_Row | 1315.776 | [1296.903, 1322.686] | 0.5176 [0.5099, 0.5194] | 0.3529 | 219.252 |
| D_codes_dedup_array | 1155.223 | [1152.194, 1159.641] | 0.4544 [0.4507, 0.4555] | 0.3099 | 192.498 |
| E_no_dedup_String_ahash | 59.419 | [59.328, 60.144] | 0.0234 [0.0232, 0.0236] | 0.0159 | 9.901 |
| F_no_dedup_codes_rows | 11.843 | [11.827, 11.877] | 0.0047 [0.0046, 0.0047] | 0.0032 | 1.974 |
| G_columnar_codes | 9.777 | [9.770, 9.780] | 0.0038 [0.0038, 0.0039] | 0.0026 | 1.629 |
| I_stable_wrapper_control | 2494.966 | [2486.003, 2505.361] | 0.9814 [0.9727, 0.9852] | 0.6692 | 415.744 |
| J_owned_clone_control | 2982.878 | [2971.416, 2989.895] | 1.1733 [1.1615, 1.1759] | 0.8001 | 497.046 |
| K_no_dedup_String_SipHash | 125.008 | [124.896, 125.314] | 0.0492 [0.0488, 0.0493] | 0.0335 | 20.831 |
| L_hashonly_direct_ahash_String_SipHash | 244.492 | [244.303, 244.932] | 0.0962 [0.0954, 0.0964] | 0.0656 | 40.740 |

## Paired contrasts

Positive savings mean the destination is faster. JSON also retains historical-A ratio CIs and median-of-paired-differences statistics.

| Change | From → to | Saved median ms (95% CI) | To/from ratio (95% CI) |
|---|---|---|---|
| growth_including_rehash | H → B | 1192.405 [1183.720, 1210.561] | 0.5310 [0.5272, 0.5387] |
| borrowed9_to_row_reference | H → C | 1226.436 [1219.760, 1248.581] | 0.5176 [0.5099, 0.5194] |
| codes_with_dedup_and_array_groups | H → D | 1386.989 [1381.476, 1407.437] | 0.4544 [0.4507, 0.4555] |
| remove_dedup_keep_String_SipHash | H → K | 2417.203 [2411.704, 2438.638] | 0.0492 [0.0488, 0.0493] |
| grouping_SipHash_to_ahash | K → E | 65.590 [64.867, 65.897] | 0.4753 [0.4741, 0.4810] |
| String_ahash_to_code_array | E → F | 47.575 [47.481, 48.304] | 0.1993 [0.1968, 0.2000] |
| row_to_columnar_codes | F → G | 2.067 [2.050, 2.107] | 0.8255 [0.8226, 0.8267] |
| legacy_wrapper_to_engine_direct | A → H | 1185.979 [1132.849, 1195.696] | 0.6819 [0.6801, 0.6933] |
| fresh_state_to_stable_state_same_wrapper | A → I | 1233.224 [1195.815, 1246.812] | 0.6692 [0.6665, 0.6764] |
| wrapper_API_vs_direct_primitive_writes | I → H | -47.246 [-70.063, -37.644] | 1.0189 [1.0151, 1.0281] |
| owned_clone_control_over_borrowed_H | J → H | 440.667 [413.123, 446.234] | 0.8523 [0.8504, 0.8610] |
| standalone_direct_ahash_cost | L → K | 119.484 [119.087, 119.932] | 0.5113 [0.5103, 0.5127] |

## Attribution supported by the controlled experiments

- **Dedup removal (H−K): 2417.203 ms**, 95.1% of H. Includes hashing, probes, key storage, growth and destruction—not String heap cloning.
- **Reservation/growth (H−B): 1192.405 ms**, 46.9% of H. Bundles allocations, entry migration and repeated rehashing; not allocation alone.
- **One hash per qualifying row (L−K): 119.484 ms.** A hash-only probe, not the exact hash fraction of H, because H also rehashes during growth and incurs table/cache interactions.
- **Wrapper API (I−H): -47.246 ms.** Contrary to an assumed wrapper penalty, the stable wrapper is slightly faster here. It retains the same seed keys, but byte-oriented versus specialized primitive writes can produce different hashes and table probe distributions. [INFERENCE] Hash distribution and code generation may explain the reversal; this experiment does not isolate them. Do not attribute the historical fresh-state control's large penalty to wrapping alone.
- **Owned materialization control (J−H): 440.667 ms.** String clones/allocations plus larger owned table entries. H itself clones no String heaps; this contrast cannot explain real-engine clone cost.
- **Grouping hasher (K−E): 65.590 ms**, dedup absent and String representation held constant.
- **String/hashed grouping to codes/array (E−F): 47.575 ms.** Combines compact rows, avoiding string hashing and direct addressing; not pure string comparison cost.
- **Columnar layout (F−G): 2.067 ms** after dedup removal and encoded grouping.

## Actual uninstrumented-worker perf profile

Profiled the retained no-feature `cargo xtask bench` worker directly, not the top-level executable (which had subsequently been overwritten by a counters build). Full SHA256, Cargo fingerprint, exact command and environment are in the attached profile provenance JSON.
- Binary: `/data/mini_linq/features/workloads/target/bench/build/release/deps/tpch_derived_retail_h01-2abb2513a75956d0`.
- SHA256: `9034fffae39970948db6d07c50d5efbb89119572950ec24ccb3c3f84f709c173`.
- `perf record -g --call-graph dwarf,16384 -e cycles:u -e page-faults -F 199`; default query configuration, RAYON_NUM_THREADS=1, actual SF1 inputs and existing DuckDB oracle. One worker warmup plus 21 checked query runs. This is a diagnostic profile, not a substitute for the interleaved M6 samples.
- Worker load_ms: 1069.997255; profiled query median_ms: 2597.293107; instrumented flag: False.
- Whole-process capture includes load. Callchains classify samples as load, query, or other; only query-classified cycles form the query role table below. [INFERENCE] Symbol-based classification is conservative and may leave inlined/unwound/truncated samples unclassified. User-cycle samples cannot measure kernel page-fault handling time; page-fault events are reported separately.

### Phase shares (weighted sampled cycles)

| Phase | Weighted cycle share | Samples |
|---|---:|---:|
| query | 98.46% | 11316 |
| load | 1.32% | 214 |
| other/unclassified | 0.22% | 37 |

### Query-only self-symbol roles

| Role | Weighted query-cycle share | Samples |
|---|---:|---:|
| dedup hashing | 40.77% | 5165 |
| dedup table/probe/growth | 21.75% | 2219 |
| group table/probe | 11.83% | 1146 |
| group hashing (SipHash) | 8.55% | 831 |
| page-fault-context sample | 8.33% | 1092 |
| fused scan/decimal/group reduction | 7.25% | 706 |
| string/key comparisons | 1.14% | 109 |
| other/unclassified | 0.38% | 48 |
| allocation/free | 0.00% | 0 |
| memory copy/zero | 0.00% | 0 |
| scan/decimal | 0.00% | 0 |

Zero standalone allocation/copy or decimal self samples do **not** mean zero allocation/decimal cost: allocator work can be inlined or sampled in page-fault contexts, and scan/decimal/reduction code is fused. Page-fault-context cycles are contextual samples, not a measurement of total kernel page-fault time.

### Top query self symbols

| Symbol | Role | Weighted query-cycle share |
|---|---|---:|
| `<cq_storage::hash_state::HashState as core::hash::BuildHasher>::hash_one::<&(&i64, &alloc::string::String, &i64, &cq_load::scalar::Decimal, i64, &cq_load::scalar::Decimal, i64, &alloc::string::String, &cq_load::scalar::Date)>` | dedup hashing | 40.77% |
| `<hashbrown::map::HashMap<(&i64, &alloc::string::String, &i64, &cq_load::scalar::Decimal, i64, &cq_load::scalar::Decimal, i64, &alloc::string::String, &cq_load::scalar::Date), (), cq_storage::hash_state::HashState>>::insert` | dedup table/probe/growth | 13.80% |
| `<hashbrown::map::HashMap<(&alloc::string::String, &alloc::string::String), (i64, i64, usize), std::hash::random::RandomState>>::rustc_entry` | group table/probe | 11.83% |
| `tpch_derived_retail_h01::query::default::run` | fused scan/decimal/group reduction | 6.55% |
| `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` | group hashing (SipHash) | 6.19% |
| `<(&i64, &alloc::string::String, &i64, &cq_load::scalar::Decimal, i64, &cq_load::scalar::Decimal, i64, &alloc::string::String, &cq_load::scalar::Date) as hashbrown::Equivalent<(&i64, &alloc::string::String, &i64, &cq_load::scalar::Decimal, i64, &cq_load::scalar::Decimal, i64, &alloc::string::String, &cq_load::scalar::Date)>>::equivalent` | dedup table/probe/growth | 5.03% |
| `<hashbrown::raw::RawTable<((&i64, &alloc::string::String, &i64, &cq_load::scalar::Decimal, i64, &cq_load::scalar::Decimal, i64, &alloc::string::String, &cq_load::scalar::Date), ())>>::reserve_rehash::<hashbrown::map::make_hasher<(&i64, &alloc::string::String, &i64, &cq_load::scalar::Decimal, i64, &cq_load::scalar::Decimal, i64, &alloc::string::String, &cq_load::scalar::Date), (), cq_storage::hash_state::HashState>::{closure#0}>` | dedup table/probe/growth | 2.92% |
| `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&(&alloc::string::String, &alloc::string::String)>` | group hashing (SipHash) | 2.35% |
| `clear_page_erms` | page-fault-context sample | 1.99% |
| `asm_exc_page_fault` | page-fault-context sample | 1.78% |
| `__memcmp_avx2_movbe` | string/key comparisons | 1.14% |
| `<core::iter::adapters::map::Map<core::iter::adapters::map::Map<core::iter::adapters::filter::Filter<core::iter::adapters::map::Map<core::iter::adapters::map::Map<core::slice::iter::Iter<(i64, i64, alloc::string::String, alloc::string::String, cq_load::scalar::Decimal, cq_load::scalar::Decimal, cq_load::scalar::Date)>, <&(i64, i64, alloc::string::String, alloc::string::String, cq_load::scalar::Decimal, cq_load::scalar::Decimal, cq_load::scalar::Date) as cq_storage::rows::Columns>::columns>, tpch_derived_retail_h01::query::no_reduce::run::{closure#1}>, tpch_derived_retail_h01::query::no_reduce::run::{closure#2}>, tpch_derived_retail_h01::query::no_reduce::run::{closure#3}>, tpch_derived_retail_h01::query::no_reduce::run::{closure#4}> as core::iter::traits::iterator::Iterator>::try_fold::<(), core::iter::adapters::map::map_try_fold<(&i64, &alloc::string::String, &i64, &cq_load::scalar::Decimal, i64, &cq_load::scalar::Decimal, i64, &alloc::string::String, &cq_load::scalar::Date), (&i64, &alloc::string::String, &i64, &cq_load::scalar::Decimal, i64, &cq_load::scalar::Decimal, i64, &alloc::string::String, &cq_load::scalar::Date), (), core::ops::control_flow::ControlFlow<(&i64, &alloc::string::String, &i64, &cq_load::scalar::Decimal, i64, &cq_load::scalar::Decimal, i64, &alloc::string::String, &cq_load::scalar::Date)>, tpch_derived_retail_h01::query::no_reduce::run::{closure#5}, core::iter::traits::iterator::Iterator::find::check<(&i64, &alloc::string::String, &i64, &cq_load::scalar::Decimal, i64, &cq_load::scalar::Decimal, i64, &alloc::string::String, &cq_load::scalar::Date), &mut tpch_derived_retail_h01::query::no_reduce::run::{closure#6}>::{closure#0}>::{closure#0}, core::ops::control_flow::ControlFlow<(&i64, &alloc::string::String, &i64, &cq_load::scalar::Decimal, i64, &cq_load::scalar::Decimal, i64, &alloc::string::String, &cq_load::scalar::Date)>>` | fused scan/decimal/group reduction | 0.70% |
| `__handle_mm_fault` | page-fault-context sample | 0.49% |
| `___perf_sw_event` | page-fault-context sample | 0.28% |
| `get_page_from_freelist` | page-fault-context sample | 0.25% |
| `do_user_addr_fault` | page-fault-context sample | 0.24% |
| `lock_vma_under_rcu` | page-fault-context sample | 0.24% |
| `rmqueue` | page-fault-context sample | 0.24% |
| `mas_walk` | page-fault-context sample | 0.17% |
| `__alloc_pages` | page-fault-context sample | 0.16% |
| `__rmqueue_pcplist` | page-fault-context sample | 0.16% |
| `handle_mm_fault` | page-fault-context sample | 0.15% |
| `__perf_sw_event` | page-fault-context sample | 0.14% |
| `handle_pte_fault` | page-fault-context sample | 0.13% |
| `alloc_pages_mpol` | page-fault-context sample | 0.12% |

### Sampled page-fault events

| Phase | Samples | Weighted event share |
|---|---:|---:|
| query | 3972 | 95.68% |
| load | 158 | 3.03% |
| other/unclassified | 41 | 1.29% |
Dedup hashing with a `reserve_rehash` ancestor alone accounts for 36.12% of query-classified weighted cycles. This is an inclusive context subdivision of the dedup-hashing self share, not an additional share.
This large rehash-context share and the H→B reservation contrast support growth/rehashing as a major cost. The much smaller L−K sequential hash-only contrast is not contradictory: table growth hashes keys reached through large, less-local table entries and also performs migration/probing. [INFERENCE] Locality and extra hash invocations explain why a sequential hash microprobe cannot stand in for in-table hash cost.

## Relation to 2566 ms compiled versus 819 ms DuckDB

The supplied historical times imply a 3.13× gap; DuckDB was not rerun here. M6 measures primary H=2542.212 ms and lower-bound G=9.777 ms. The actual retained no-feature worker was profiled separately above. Do not subtract microbenchmark contrasts directly from the historical 2566 ms: build flags/source revisions, seed choice, allocator/cache behavior and measurement scope differ.

[INFERENCE] The verified primary key permits dedup elimination, and code-domain grouping eliminates hashing work that compression alone cannot. The controlled contrasts quantify target-specific opportunities, not the exact end-to-end share of the historical engine–DuckDB gap. The counter-instrumented perf capture must not be used as a profile of uninstrumented execution.

## Exact result and environment

```json
[
  [
    3773410700,
    5658655440073,
    1478493
  ],
  [
    0,
    0,
    0
  ],
  [
    99141700,
    148750471038,
    38854
  ],
  [
    7447604000,
    11170172969774,
    2920374
  ],
  [
    3771975300,
    5656804138090,
    1478870
  ],
  [
    0,
    0,
    0
  ]
]
```

Group order: A/F, A/O, N/F, N/O, R/F, R/O. Triples: quantity cents, extendedprice cents, count. Empty groups are zeros. The complete nonempty result also matched the independent existing DuckDB SF1 oracle exactly.
1 accepted matrix; 68 rejected/busy records. Full round snapshots and rejected samples remain in JSON.
Accepted UTC window: 2026-10-29T05:20:47Z through 2026-10-29T05:27:49Z.

## Reproduction

```sh
python3 experiments/storage-bench/cpu/extract_strings.py
nice -n 19 env CARGO_TARGET_DIR=/data/mini_linq/experiments/storage-bench/cpu/target RUSTFLAGS='-C target-cpu=native' cargo build --release -j1 --manifest-path experiments/storage-bench/cpu/Cargo.toml --bin h01
experiments/storage-bench/cpu/target/release/h01 --small
# Only in a quiet authorized window; do not overlap these runs:
experiments/storage-bench/cpu/target/release/h01 --machine-quiet
python3 experiments/storage-bench/cpu/profile_h01.py
python3 experiments/storage-bench/cpu/analyze_h01_profile.py
python3 experiments/storage-bench/cpu/summarize_h01.py
```

## M6b: order-verified run distinct versus hash and packed-sort distinct

**No uniqueness assumption.** S1 checks only nondecreasing lexicographic `(l_orderkey,l_linenumber)` in one sequential resident-data pass. Equal prefixes are permitted. Within every equal-prefix run, S2/S3 compare a candidate full source tuple against all earlier tuples in that run. Thus X,Y,X suppresses the second X; merely comparing adjacent full tuples would be incorrect. Worst-case work inside a length-L run is quadratic; this is a measured choice for this input, not a general asymptotic improvement over hashing.

**S1 result:** nondecreasing verification passed. Uniqueness is neither tested nor assumed. If verification fails, S2 falls back to global String/hash distinct, S3 to global reserved code/hash distinct; H/R also use the global serial fallback instead of unsafe independent chunk distinct. Tests covered nonadjacent duplicates within a prefix run, a run crossing 65,536-row task boundaries, excluded dates, and unsorted fallback.

t24 uses fixed 65,536-row tasks extended to complete equal-prefix runs. H/R use the same partitioning as S2/S3, so duplicates cannot straddle independent sets. **t24 H is this common parallelized kernel, not a claim about the generated engine’s parallel configuration.** t1 uses the full input in one task. S1 and packing-bound construction remain single-thread sequential load-time passes even when shown in the t24 experiment group; their facts are reused by query samples.

P/P′ require no input ordering: filter, encode all seven source columns into an injective u128, sort, adjacent-unique, then group directly from packed fields. All seven fields participate in equality, not just the prefix. Load-time min/max bounds describe representability; this input fits u128. P is stable LSD radix with 8-bit digits; t24 uses fixed-chunk histograms, exclusive per-bucket/chunk offsets and parallel scatter. P′ uses `sort_unstable` / Rayon `par_sort_unstable` (Rayon controls its comparison-sort scheduling). Adjacent-unique emits a selection mask rather than copying a compacted key array; grouping consumes only marked keys.

| Field | Base | Width bits | Shift |
|---|---:|---:|---:|
| orderkey | 1 | 23 | 0 |
| linenumber | 1 | 3 | 23 |
| returnflag_code | 0 | 2 | 26 |
| linestatus_code | 0 | 1 | 28 |
| quantity_cents | 100 | 13 | 29 |
| price_cents | 90100 | 24 | 42 |
| shipdate_days | 8036 | 12 | 66 |

Total: **78 bits** in a 128-bit word; 10 radix passes. Bounds are computed over all resident input rows, before predicate filtering.

**Protocol:** five warmups, 21 samples, rotated interleaving, paired-bootstrap ratios to the H measured in the same thread-count group. Exact per-sample answers matched the independent DuckDB SF1 oracle. No changes were selected by observed performance. Input construction and load-fact calculation are excluded from query times and reported separately. Query allocations, scratch/mask creation and destruction are included; phase residuals include dispatch/timer and final vector destruction. The existing environment checks record clocks/load/top processes/GPU tenants after each round and repeat contaminated groups.

Ratios to DuckDB use the **supplied historical t1 819 ms** as a fixed denominator, not a newly measured control or a paired comparison. Their CIs reflect only this experiment’s uncertainty; t24/DuckDB-t1 ratios are deliberately cross-thread reference ratios.

### M6b query measurements

| t | Variant | Median ms | 95% CI ms | Ratio to same-t H (95% CI) | Ratio to DuckDB t1 (95% CI) |
|---:|---|---:|---|---|---|
| 1 | H_hash_String_SipHash | 2541.385 | [2519.265, 2550.081] | 1.0000 [1.0000, 1.0000] | 3.1030 [3.0760, 3.1137] |
| 1 | R_reserved_hash_codes | 660.442 | [647.745, 675.868] | 0.2599 [0.2559, 0.2669] | 0.8064 [0.7909, 0.8252] |
| 1 | S2_ordered_String_ahash | 76.135 | [75.962, 76.366] | 0.0300 [0.0299, 0.0303] | 0.0930 [0.0928, 0.0932] |
| 1 | S3_ordered_codes_array | 17.520 | [17.472, 17.556] | 0.0069 [0.0069, 0.0070] | 0.0214 [0.0213, 0.0214] |
| 1 | P_packed_radix | 463.337 | [461.303, 464.485] | 0.1823 [0.1813, 0.1842] | 0.5657 [0.5633, 0.5671] |
| 1 | Pprime_packed_comparison | 258.716 | [258.222, 259.000] | 0.1018 [0.1015, 0.1027] | 0.3159 [0.3153, 0.3162] |
| 24 | H_hash_String_SipHash | 139.509 | [133.771, 141.130] | 1.0000 [1.0000, 1.0000] | 0.1703 [0.1633, 0.1723] |
| 24 | R_reserved_hash_codes | 29.324 | [28.565, 30.457] | 0.2102 [0.2060, 0.2180] | 0.0358 [0.0349, 0.0372] |
| 24 | S2_ordered_String_ahash | 12.953 | [12.756, 13.660] | 0.0928 [0.0912, 0.0980] | 0.0158 [0.0156, 0.0167] |
| 24 | S3_ordered_codes_array | 3.297 | [3.154, 3.399] | 0.0236 [0.0231, 0.0247] | 0.0040 [0.0039, 0.0042] |
| 24 | P_packed_radix | 130.495 | [129.396, 131.356] | 0.9354 [0.9180, 0.9771] | 0.1593 [0.1580, 0.1604] |
| 24 | Pprime_packed_comparison | 91.817 | [90.988, 92.172] | 0.6581 [0.6482, 0.6871] | 0.1121 [0.1111, 0.1125] |

### Separate load-time representation checks

| Execution configuration | Pass | Actual threads | Median ms | 95% CI ms |
|---:|---|---:|---:|---|
| 1 | S1_load_order_verification | 1 | 20.967 | [20.880, 21.043] |
| 1 | P_load_FOR_bounds | 1 | 14.157 | [14.131, 14.238] |
| 24 | S1_load_order_verification | 1 | 22.635 | [21.425, 23.492] |
| 24 | P_load_FOR_bounds | 1 | 14.395 | [14.281, 14.933] |

These are resident-data construction costs, not file I/O timings. They are paid once per immutable loaded representation, not once per query. S2/S3 depend on S1; P/P′ depend on the FOR-bound pass. H/R t24 also depend on S1 for the common prefix-aligned partitioning.

### Packed sort / unique / group phase times

| t | Variant | Phase | Median ms | 95% CI ms |
|---:|---|---|---:|---|
| 1 | P_packed_radix | encode_and_allocate | 89.259 | [87.795, 89.373] |
| 1 | P_packed_radix | sort | 329.180 | [328.772, 330.407] |
| 1 | P_packed_radix | adjacent_unique_mask | 4.952 | [4.947, 4.959] |
| 1 | P_packed_radix | group_packed_codes | 32.937 | [32.922, 32.960] |
| 1 | P_packed_radix | outer overhead and final drop | 6.379 | — |
| 1 | Pprime_packed_comparison | encode_and_allocate | 87.233 | [86.922, 87.464] |
| 1 | Pprime_packed_comparison | sort | 126.836 | [126.765, 126.930] |
| 1 | Pprime_packed_comparison | adjacent_unique_mask | 5.210 | [5.198, 5.215] |
| 1 | Pprime_packed_comparison | group_packed_codes | 32.926 | [32.905, 32.947] |
| 1 | Pprime_packed_comparison | outer overhead and final drop | 6.293 | — |
| 24 | P_packed_radix | encode_and_allocate | 52.473 | [51.348, 52.837] |
| 24 | P_packed_radix | sort | 66.029 | [65.304, 66.420] |
| 24 | P_packed_radix | adjacent_unique_mask | 2.384 | [2.354, 2.416] |
| 24 | P_packed_radix | group_packed_codes | 3.024 | [3.021, 3.060] |
| 24 | P_packed_radix | outer overhead and final drop | 6.932 | — |
| 24 | Pprime_packed_comparison | encode_and_allocate | 54.966 | [54.023, 55.802] |
| 24 | Pprime_packed_comparison | sort | 24.392 | [24.256, 24.842] |
| 24 | Pprime_packed_comparison | adjacent_unique_mask | 1.913 | [1.882, 1.949] |
| 24 | Pprime_packed_comparison | group_packed_codes | 3.812 | [3.790, 3.834] |
| 24 | Pprime_packed_comparison | outer overhead and final drop | 6.311 | — |

Phase medians need not sum to the median total; every raw phase sample is retained in JSON.

### M6b interpretation

- **t1 fastest median:** S3_ordered_codes_array at 17.520 ms (0.0069× same-t H). This is a median ranking; the individual CIs above determine whether differences are resolved.
- t1: radix total/comparison-sort total = 1.791; radix/reserved-code-hash total = 0.702. These descriptive median ratios do not carry a separate significance claim.
- t1: comparison sort also wins the isolated sort phase (126.836 versus radix 329.180 ms), contrary to assuming radix must win integer-key sorting. [INFERENCE] Ten radix passes, scratch initialization/scatter traffic, and the library sort's implementation/input sensitivity can explain this result; no counter-level attribution was collected for these variants.
- t1: S1 median + S3 median = 38.487 ms as a first-query cost illustration; not a separately timed sum or a CI estimate.
- **t24 fastest median:** S3_ordered_codes_array at 3.297 ms (0.0236× same-t H). This is a median ranking; the individual CIs above determine whether differences are resolved.
- t24: radix total/comparison-sort total = 1.421; radix/reserved-code-hash total = 4.450. These descriptive median ratios do not carry a separate significance claim.
- t24: comparison sort also wins the isolated sort phase (24.392 versus radix 66.029 ms), contrary to assuming radix must win integer-key sorting. [INFERENCE] Ten radix passes, scratch initialization/scatter traffic, and the library sort's implementation/input sensitivity can explain this result; no counter-level attribution was collected for these variants.
- t24: S1 median + S3 median = 25.933 ms as a first-query cost illustration; not a separately timed sum or a CI estimate.
- S2/S3 measure elimination of a global hash set under a verified ordering fact, **not** elimination of DISTINCT by presumed key uniqueness. Nonadjacent duplicate payloads within a prefix run remain correct.
- P/P′ retain fully general order-independent distinct semantics within the verified 128-bit packing domain. Their sort/unique/group costs must be weighed against reserved hashing rather than against an illegal no-distinct shortcut.
- At t24, R exploits verified prefix routing into independent sets while P/P′ perform a global order-independent sort. Thus the t24 ratios do not establish a winner between two unrestricted unordered-input parallel algorithms. P/P′ also include standard collection/allocation costs in encoding; these are measured implementations, not claims of optimal radix or packing performance.
- [INFERENCE] Packing can reduce key footprint and random pointer accesses, while radix costs multiple full passes and scratch writes. Comparison sort can respond differently to this input’s value distribution. These mechanisms explain candidate tradeoffs, not measured hardware-counter attribution.
- [INFERENCE] t24 changes scheduling, memory bandwidth and partition working-set size; scaling is not attributable solely to the number of CPU cores.

### M6b environment and reproduction

2 accepted thread-count groups; 36 busy/rejected records. Small correctness cases and every retained sample were exact.
t1: 2026-10-29T06:01:16Z through 2026-10-29T06:03:03Z.
t24: 2026-10-29T06:03:03Z through 2026-10-29T06:03:17Z.

```sh
nice -n 19 env CARGO_TARGET_DIR=/data/mini_linq/experiments/storage-bench/cpu/target RUSTFLAGS='-C target-cpu=native' cargo build --release -j1 --manifest-path experiments/storage-bench/cpu/Cargo.toml --bin h01b
experiments/storage-bench/cpu/target/release/h01b --small
# Only in a quiet authorized measurement window:
experiments/storage-bench/cpu/target/release/h01b --machine-quiet
python3 experiments/storage-bench/cpu/summarize_h01b.py
```

Supplemental raw matrix: `cpu-h01-order-distinct-2026-10-29.json`; it is also embedded as `m6b` in the main M6 JSON.
