# GPU DOOP vs recorded SRDatalog

Read-only capture of the supplied M0 `bin/base/srd-doop-gpu`, `GPU_DATALOG_PHASES=1 --gpu-only`; no compilation. GPU: RTX 6000 Ada, 142 SMs, nominal 960.096 GB/s. Commands/environment: `capture-manifest.json`; raw timelines: `xalan.nsys-rep`, `batik.nsys-rep` and SQLite exports. Logs retain program phase timings.

## Evidence

| Phase / source | xalan | batik |
|---|---:|---:|
| Supplied unprofiled M0 fixpoint | ~1.40 s | ~4.31 s |
| This nsys fixpoint | 1.761386 s | 4.970259 s |
| This nsys setup / output | 0.165526 / 5.072927 s | 0.149542 / 6.131787 s |
| SRD fullclock fixpoint, recorded range | 1.008194–1.021628 s | 1.818340–2.101104 s |
| SRD baseline fixpoint, recorded range | 1.464220–1.466372 s | 2.981322–2.986936 s |
| SRD fullclock load / prepare | 0.172339–0.178141 / 0.015938–0.017171 s | 0.169389–0.175168 / 0.015791–0.017175 s |
| SRD fullclock counts | 0.130–0.208 ms | 0.162–0.264 ms |

Do not treat profiled versus historical timings as a paired experiment: capture changes timings and the historical runs have different clock configurations. The supplied unprofiled values imply approximately 1.38x / 2.36x the fullclock SRD medians, but xalan is already faster than the older SRD baseline. `srd-comparison.json` also records the slower SRD `current` reports (xalan 2.414–2.441 s; batik 6.070–6.129 s). `/data/doop-bench/srd-measurements.json` explicitly describes CPU correctness/cardinality runs, not GPU comparative performance.

### Additive fixpoint timeline attribution

Times in ms; parentheses are percentage of profiled fixpoint wall time. GPU categories are kernel-symbol based, not nested host API sums.

| Cost | xalan | batik |
|---|---:|---:|
| Generated rule/join kernels | 701.499 (39.83%) | 3399.865 (68.40%) |
| Sort/unique, including lane gathers and layout sorts | 271.321 (15.40%) | 319.327 (6.42%) |
| Set difference/admission | 14.841 (0.84%) | 47.216 (0.95%) |
| Merge/layout kernels excluding their sorts | 21.993 (1.25%) | 50.077 (1.01%) |
| Other kernels | 0.553 (0.03%) | 0.557 (0.01%) |
| Device copies | 36.975 (2.10%) | 55.021 (1.11%) |
| Device memset | 13.409 (0.76%) | 22.248 (0.45%) |
| GPU-idle time inside host synchronize/D2H APIs | 254.247 (14.43%) | 293.765 (5.91%) |
| GPU-idle time inside launch APIs | 94.270 (5.35%) | 181.982 (3.66%) |
| GPU-idle time inside other CUDA APIs | 151.185 (8.58%) | 255.396 (5.14%) |
| GPU-idle time outside CUDA APIs | 201.094 (11.42%) | 344.804 (6.94%) |

**GPU idle:** 39.79% / 21.65%. **Kernel concurrency:** only stream 7; zero overlapping kernel duration. 49,212 / 80,280 fixpoint kernel launches. Copies total 2.055 / 5.672 GB, overwhelmingly D2D (2.054 / 5.671 GB), not bulk host data transfers. Fixpoint D2H calls: 17,365 / 24,067; stream synchronizations: 17,412 / 24,114. Whole launch API duration is 138.682 / 266.849 ms; the table includes only its GPU-idle portion. D2H API wall durations (927.960 / 3702.506 ms) overlap GPU work and MUST NOT be added to kernel time.

**Top 20 kernels with calls, ms, kernel share and fixpoint share:** `xalan.top20.csv`, `batik.top20.csv`. Full rankings and CUDA API totals: `*.summary.json`. Selected dominant kernels:

| Kernel suffix (dispatch 0 unless noted) | xalan ms (% kernels) | batik ms (% kernels) |
|---|---:|---:|
| `e829d542d2ee2f48_56` | 151.125 (14.96%) | 620.672 (16.26%) |
| `cb1bcae3794b49d2_63` — VPT_LoadArray | 68.179 (6.75%) | 1383.271 (36.24%) |
| `7ab0e49757f519db_33` | 89.448 (8.85%) | 69.200 (1.81%) |
| `0427af5994332909_62` — VPT_LoadArray | see full ranking | 135.856 (3.56%) |
| `device_radix_sort_onesweep_pairs` | 132.436 (13.11%) | 106.966 (2.80%) |

## Three largest attributable costs / likely gaps

1. **Join expansion, especially batik array loads.** Generated kernels consume 3.400 s on batik. Two kernel programs mapped to `VPT_LoadArray` consume 1.519 s, 30.56% of fixpoint wall time; the largest alone launches 384 times. Mapping is recorded in `experiments/gpu/compare/p32-doop-advice.json:1`, `counter_variants` (exact matching program hashes, historical `decompose` plan). The six-relation rule is at `features/datalog/doop/srd/src/generated.rs:517`; alternative order/early smallest advice is at `experiments/gpu/srd-doop/src/lib.rs:50–54`. [INFERENCE] Repeated candidate expansion and late compatibility filtering are the leading batik algorithmic gap. Build diagnostics also report register spilling for `cb1b...` in `experiments/gpu/target/release/.fingerprint/srd-doop-gpu-37e5d5d095ffaabc/output-lib-srd_doop_gpu:25`; no DOOP hardware counters were collected, so the spill contribution is not quantified.
2. **Serialized host-controlled work and launch gaps.** 700.795 / 1075.947 ms of idle GPU time, including 254.247 / 293.765 ms inside sync/readback APIs. Admission downloads unique and fresh counts: `experiments/gpu/datalog-runtime/src/runs.rs:117–123,291–317`. [INFERENCE] Host count-dependent admission/allocation and many tiny launches prevent overlap; this is the largest xalan opportunity. The idle intersection is measured, but exact CPU execution versus driver/profiler/scheduler overhead within residual gaps is not observable from this capture.
3. **Repeated radix canonicalization/layout sorting.** 271.321 / 319.327 ms, with 480 / 1040 onesweep-pair passes and thousands of key-sort launches. Wide packed rows sort stable least-significant 64-bit lanes (`experiments/gpu/datalog-runtime/src/runs.rs:244–290`); layout-specific unique runs also sort (`:193–206`). [INFERENCE] Whole-word lane passes plus temporary permutations explain unnecessary work compared with a narrower/shared-key strategy. Admission and merges themselves are much smaller than sorting; optimizing only set difference misses the main cost.

## Scope / attribution limits

All code paths above are relative to `/data/mini_linq-wt/m0-baseline/`. Existing binaries emit no NVTX phase/rule ranges. [INFERENCE] Fixpoint boundaries are reconstructed from the last `cuStreamSynchronize` before the first generated rule kernel and the later synchronize whose separation matches the printed duration: xalan 916201218–2677587476 ns; batik 888453287–5858712450 ns. Agreement is 0.258 / 0.163 microseconds. This matches phase synchronization in `datalog-runtime/src/phases.rs:27–35` and setup/fixpoint/output placement in `datalog/src/print.rs:433–437`.

Classification uses generated dispatches as rule kernels; radix/gather/adjacent-unique as sort; absent plus its subsequent selection chain as admission; merge/partition/permute as layout maintenance. Shared selection kernels and layout radix work cannot be perfectly separated without instrumentation: the sort bucket includes layout sorts, and [INFERENCE] selection-chain attribution uses the preceding absent/unique producer. No concurrent kernels were observed. Idle is the complement of the union of kernels, memcpy and memset, then intersected with host API intervals, so the table does not double-count waits.

The SRD `report.json` files record only whole fixpoint and load/init/prepare/counts/shutdown stages, **not per-rule or within-fixpoint phase times**. Thus these are measured costs in ours and likely causes, not proven per-rule SRD deltas. Ours materializes output outside fixpoint (5–6 s), whereas SRD uses cardinality-only/no-export reports; output phases are not directly comparable and are excluded from the fixpoint comparison.
